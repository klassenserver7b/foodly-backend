---
sessionID: ses_e56d3568dffeuufkvPs8oL6ruN
baseMessageCount: 75
updatedAt: 2026-11-06T00:15:49.267Z
version: 1.0
date_created: 2026-11-06
owner: agent
tags: [spec, diagnostic]
consumedState: b0e44da91f05fb1fc3db798bc55f5225b3269615dbc1d4f23d796143e63036d3
ideaHash: f597ae9e8025ee37ea38c3a6e70485531e299b3d87e7c7b6796b13273df087c5
---

# real-auth-spec

## Current spec

# Introduction

This specification defines the implementation of **real user authentication** for the Foodly backend (Rust/Axum/PostgreSQL), replacing the current mock auth middleware. It covers user registration, login, token refresh, the real auth middleware, and the CORS configuration fix for the `Authorization` header.

The current `auth_middleware` in `src/lib.rs` accepts **any** Bearer token and inserts a hardcoded `mock_user_id = 1` into request extensions. No `/auth/*` routes exist. CORS uses `allow_headers(Any)` which, per the Fetch standard, does **not** cover `Authorization` — cross-origin requests with auth headers fail at preflight (the frontend works around this in dev via the Vite proxy).

## 1. Purpose & Scope

**Intended audience:** Backend maintainers implementing the auth system.

**In scope:**
- `POST /api/v1/auth/register` — create user account
- `POST /api/v1/auth/login` — authenticate, receive tokens
- `POST /api/v1/auth/refresh` — exchange refresh token for new token pair
- `POST /api/v1/auth/logout` — revoke one refresh token (logout from one device)
- `DELETE /api/v1/auth/sessions` — revoke all refresh tokens (logout from all devices)
- Real `auth_middleware` that validates actual tokens and extracts the real `user_id`
- CORS fix: explicit `allow_headers([AUTHORIZATION, CONTENT_TYPE])` + configurable origin allow-list
- Password hashing (argon2id) for the existing `users.password_hash` column
- Refresh token storage in the existing `refresh_tokens` table (hashed)
- Password policy: minimum 8 characters, no composition rules (NIST SP 800-63B)
- Rate limiting: 3 modes (direct / proxy-header / disabled) via ENV / Config

**Out of scope (deferred):**
- WebSocket ticket endpoint (`ws_tickets` table exists in migration 012 but no endpoint — deferred to WS phase)
- Password reset flows, email verification, OAuth/social login
- The `GET /users/me` endpoint (already exists, will consume real auth)

**Assumptions:**
- The `refresh_tokens` and `ws_tickets` tables (migration 012) already exist
- The `users` table (migration 001) already has a `password_hash` column
- The frontend (Foodly-FE) will consume these endpoints for its login page

## 2. Definitions

- **JWT**: JSON Web Token — signed, self-contained token format (RFC 7519)
- **HS256**: HMAC-SHA256 JWT signing algorithm
- **Access token**: Short-lived token (e.g., 15 min) used to authenticate API requests
- **Refresh token**: Long-lived token (e.g., 30 days) used to obtain new access tokens
- **Token rotation**: Issuing a new refresh token each time the refresh endpoint is used; the old one is invalidated
- **CITEXT**: PostgreSQL case-insensitive text type (used for `users.email`)
- **argon2id**: OWASP-recommended password hashing algorithm (memory-hard, GPU-resistant)
- **SPA**: Single-Page Application (the Foodly frontend)

## 3. Requirements, Constraints & Guidelines

### Requirements
- **REQ-001**: `POST /api/v1/auth/register` must accept `{name, email, password}`, create a user with an argon2id-hashed password, and return `201 Created` with the user object and token pair
- **REQ-002**: `POST /api/v1/auth/login` must accept `{email, password}`, validate credentials, and return `200 OK` with the user object and token pair
- **REQ-003**: `POST /api/v1/auth/refresh` must accept `{refreshToken}`, validate it against the `refresh_tokens` table (hash match + not expired), rotate it (delete old, insert new), and return `200 OK` with a new token pair
- **REQ-004**: The real `auth_middleware` must extract the Bearer token from the `Authorization` header, validate the JWT (signature + expiry), extract `user_id` from the `sub` claim, and insert it as `Extension<i32>`
- **REQ-005**: Auth endpoints (`/auth/register`, `/auth/login`, `/auth/refresh`) must NOT be wrapped by the auth middleware — they are public
- **REQ-006**: CORS must explicitly allow `Authorization` and `Content-Type` headers (not `*`, which doesn't cover `Authorization` per the Fetch spec)
- **REQ-007**: Registration must return `409 Conflict` if the email already exists (CITEXT is case-insensitive unique)
- **REQ-008**: Login must return `401 Unauthorized` with a generic message for invalid email OR wrong password (no user enumeration)
- **REQ-009**: Registration must enforce a password policy: minimum 8 characters, no composition rules (NIST SP 800-63B); return `422 Unprocessable` if violated
- **REQ-010**: Auth endpoints (`/auth/register`, `/auth/login`, `/auth/refresh`) must have rate limiting: 3 modes (direct / proxy-header / disabled) via `RATE_LIMIT_MODE` env var; default 5 requests per minute per IP; burst from a single IP → `429 Too Many Requests` after limit exceeded

### Security Constraints
- **SEC-001**: Passwords must be hashed with argon2id (default params: memory 19456 KiB, iterations 2, parallelism 1) before storage — never store plaintext
- **SEC-002**: Refresh tokens must be stored as SHA-256 hashes in the `refresh_tokens.token_hash` column — never store the raw token
- **SEC-003**: The JWT signing secret must come from an environment variable (`JWT_SECRET`), minimum 32 characters
- **SEC-004**: Login failures must return a generic `invalid credentials` message — do not reveal whether the email exists
- **SEC-005**: All auth endpoints must be served over HTTPS in production (deployment concern, not code)
- **SEC-006**: Rate limiting must be configurable via ENV (`RATE_LIMIT_MODE=direct|proxy|disabled`); in `proxy` mode, only trust `X-Real-IP` / `X-Forwarded-For` from `TRUSTED_PROXIES`

### System Constraints
- **CON-001**: Must use the existing stack: Axum, diesel-async, PostgreSQL, tower-http — no new frameworks
- **CON-002**: Must not break the existing API contract for endpoints that already work
- **CON-003**: New crate dependencies must be approved before adding to `Cargo.toml`
- **CON-004**: Must integrate with the existing `AppError` enum and error envelope `{"error": {"code", "message"}}`
- **CON-005**: Rate limiting must use a tower-compatible middleware (e.g., `tower_governor`); no new frameworks

### Guidelines
- **GUD-001**: Follow existing code patterns — `Extension(user_id): Extension<i32>` for auth, `AppError` for errors, `diesel-async` for DB
- **GUD-002**: All timestamps in API responses must be ISO 8601 UTC strings
- **GUD-003**: Keep handler functions thin — put business logic in `src/services/auth.rs`
- **GUD-004**: Rate limiting configuration should be documented in the deployment guide

## 4. Interfaces & Data Contracts

### POST /api/v1/auth/register

**Request:**
```json
{
  "name": "Kolja",
  "email": "kolja@example.com",
  "password": "hunter2"
}
```

**Response `201 Created`:**
```json
{
  "user": {
    "id": 1,
    "name": "Kolja",
    "email": "kolja@example.com",
    "profilePicture": null
  },
  "accessToken": "eyJ...",
  "refreshToken": "dGhpcyBpcyBhIHJlZnJlc2g..."
}
```

**Errors:**
- `409 Conflict` — email already registered
- `422 Unprocessable` — validation failure (missing fields, weak password)

---

### POST /api/v1/auth/login

**Request:**
```json
{
  "email": "kolja@example.com",
  "password": "hunter2"
}
```

**Response `200 OK`:**
```json
{
  "user": { "id": 1, "name": "Kolja", "email": "kolja@example.com", "profilePicture": null },
  "accessToken": "eyJ...",
  "refreshToken": "dGhpcyBpcyBhIHJlZnJlc2g..."
}
```

**Errors:**
- `401 Unauthorized` — invalid email or password (indistinguishable — prevents user enumeration)
- `422` — malformed JSON

---

### POST /api/v1/auth/refresh

**Request:**
```json
{
  "refreshToken": "dGhpcyBpcyBhIHJlZnJlc2g..."
}
```

**Response `200 OK`:**
```json
{
  "accessToken": "eyJ...(new)...",
  "refreshToken": "(new)..."
}
```

**Errors:**
- `401 Unauthorized` — refresh token expired, revoked, or not found

---

### Auth middleware contract

- Extracts `Bearer <token>` from `Authorization` header
- Decodes JWT, verifies HS256 signature against `JWT_SECRET`
- Verifies `exp` claim (not expired)
- Extracts `sub` claim → `user_id`
- Inserts `user_id: i32` into request extensions (matching current pattern)
- Returns `401 Unauthorized` for missing/invalid/expired tokens

### Database schema (existing, for reference)

**`refresh_tokens`** (migration 012):
```sql
CREATE TABLE refresh_tokens (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

## 5. Acceptance Criteria

- **AC-001**: Given a valid registration payload, When `POST /auth/register`, Then `201 Created` with user object + access token + refresh token
- **AC-002**: Given an email that already exists (case-insensitive via CITEXT), When `POST /auth/register`, Then `409 Conflict`
- **AC-003**: Given a valid email + correct password, When `POST /auth/login`, Then `200 OK` with user + token pair
- **AC-004**: Given a valid email + wrong password, When `POST /auth/login`, Then `401 Unauthorized`
- **AC-005**: Given a valid refresh token, When `POST /auth/refresh`, Then `200 OK` with new access + refresh tokens; the old refresh token is invalidated
- **AC-006**: Given an expired/revoked/unknown refresh token, When `POST /auth/refresh`, Then `401 Unauthorized`
- **AC-007**: Given a valid access token, When making any authenticated request, Then the real `user_id` from the JWT is used (not mock 1)
- **AC-008**: Given no/invalid/expired access token, When making any authenticated request, Then `401 Unauthorized`
- **AC-009**: Given a cross-origin request with an `Authorization` header, When preflight arrives, Then CORS explicitly allows the header and the request succeeds
- **AC-010**: Given a user registered via `POST /auth/register`, When `POST /auth/login` with the same credentials, Then login succeeds
- **AC-011**: Given a password shorter than 8 characters at registration, Then `422 Unprocessable` is returned
- **AC-012**: Given more than 5 auth requests per minute from the same IP, Then `429 Too Many Requests` is returned

## 6. Test Automation Strategy

**Test framework:** Rust's built-in `#[tokio::test]` + `tower::ServiceExt::oneshot` (existing pattern in `tests/api_tests.rs`)

**Mocking approach:** No mocking needed — integration tests use real PostgreSQL with fresh, isolated per-test databases (existing harness)

**Test types:**
- **Unit tests** (in-module `#[cfg(test)]`): Payload validation, argon2 hash/verify roundtrip, JWT encode/decode
- **Integration tests** (`tests/api_tests.rs`): Full auth flow — register → login → refresh → authenticated request

**Key test scenarios:**
- Register: happy path, duplicate email (409), weak password (422)
- Login: correct creds, wrong password (401), non-existent email (401, same as wrong password)
- Refresh: valid token (200 + new pair), expired token (401), reused rotated token (401)
- Middleware: valid token → correct user_id; invalid/expired/missing token → 401
- CORS: preflight `OPTIONS` with `Access-Control-Request-Headers: authorization` → correct `Access-Control-Allow-Headers` response
- Rate limiting: 6 rapid auth requests from same IP → first 5 succeed, 6th returns 429

## 7. Rationale & Context

**Why JWT access tokens + opaque refresh tokens:**
- JWTs allow stateless verification (no DB lookup per request)
- Opaque refresh tokens in DB allow revocation and rotation
- This is the industry-standard pattern for SPA authentication

**Why argon2id for passwords:**
- OWASP's current recommendation
- Memory-hard, resistant to GPU attacks
- No password length limitations (unlike bcrypt's 72-byte limit)

**Why CORS needs explicit headers:**
- `Access-Control-Allow-Headers: *` does NOT cover `Authorization` per the Fetch standard
- The `Authorization` header must be explicitly listed

**Why rate limiting on auth endpoints:**
- Brute-force protection: login/register are high-value targets for credential stuffing
- Three modes via `RATE_LIMIT_MODE` env var support different deployment topologies:
  - `direct` — rate limit using direct socket IP (only our server)
  - `proxy` — rate limit using `X-Real-IP` / `X-Forwarded-For` from trusted proxies (nginx or non-nginx)
  - `disabled` — reverse proxy handles rate limiting, none on our side
- Configurable via `RATE_LIMIT_PER_MINUTE` and `TRUSTED_PROXIES` env vars

**Alternative approaches considered:**
- Fully opaque tokens (all in DB): requires DB lookup per request, adds latency
- Pure JWT (no refresh): no revocation path, longer token life needed
- bcrypt instead of argon2: battle-tested but dated (72-byte limit, no memory hardness)

## 8. Dependencies & External Integrations

- **EXT-001**: `jsonwebtoken` crate — for JWT signing/verification (HS256)
- **EXT-002**: `argon2` crate — for password hashing (argon2id)
- **EXT-003**: `rand` crate — for generating cryptographically secure random tokens
- **EXT-004**: `sha2` crate (already in Cargo.toml) — for hashing refresh tokens
- **EXT-005**: `chrono` crate (already in Cargo.toml) — for token expiry timestamps
- **EXT-006**: `tower_governor` crate — for rate limiting on auth endpoints

**New environment variables:**
```
JWT_SECRET=<random 64+ char string>
JWT_EXPIRY_MINUTES=15
REFRESH_TOKEN_EXPIRY_DAYS=30
CORS_ALLOWED_ORIGINS=http://localhost:5173,https://app.foodly.com
RATE_LIMIT_MODE=direct
RATE_LIMIT_PER_MINUTE=5
TRUSTED_PROXIES=127.0.0.1,::1
```

## 9. Examples & Edge Cases

### Registration with duplicate email (case-insensitive)
```bash
# First registration
curl -X POST http://localhost:8080/api/v1/auth/register \
  -H "Content-Type: application/json" \
  -d '{"name": "Kolja", "email": "Kolja@Example.com", "password": "hunter2"}'

# Second registration with same email (different case) — should 409
curl -X POST http://localhost:8080/api/v1/auth/register \
  -H "Content-Type: application/json" \
  -d '{"name": "Other", "email": "kolja@example.com", "password": "hunter2"}'
```

### Login flow (happy path)
```bash
curl -X POST http://localhost:8080/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email": "kolja@example.com", "password": "hunter2"}'
```

### Authenticated request with real token
```bash
curl http://localhost:8080/api/v1/users/me \
  -H "Authorization: Bearer <access_token>"
```

### Edge cases
- **Email normalization**: CITEXT makes email uniqueness case-insensitive
- **Password minimum length**: Enforce 8+ characters (return 422 if shorter)
- **Token expiry boundary**: JWT `exp` claim checked with clock skew tolerance (±60s)
- **Concurrent refresh**: Two simultaneous refresh requests with the same token — one succeeds, one 401s
- **Empty request body**: Return 422, not 500

## 10. Validation Criteria

A valid implementation must pass:
1. All existing tests in `tests/api_tests.rs` (updated for real auth)
2. New integration tests covering the full auth flow (see Section 6)
3. Manual verification: `docker compose up -d && cargo run`, then test with curl
4. `cargo clippy` passes with no warnings
5. `cargo check` succeeds

## 11. Related Specifications / Further Reading

- **Issue #15**: https://github.com/klassenserver7b/foodly-backend/issues/15
- **API Design doc**: `resources/md/02_API_Design.md` (lines 66–167 cover auth endpoints)
- **Architecture Overview**: `resources/md/01_Architecture_Overview.md`
- **Frontend repo**: https://github.com/P1umPudding/Foodly-FE (see `docs/backend-questions.md`)
- **Fetch spec on `Access-Control-Allow-Headers: *`**: https://fetch.spec.whatwg.org/#http-access-control-allow-headers
- **OWASP Password Storage Cheat Sheet**: https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html

## Q&A history

Q: What token strategy should the auth system use?
A: JWT access tokens (15 min, stateless HS256) + opaque refresh tokens (SHA-256 hashed in the existing refresh_tokens table) — industry standard for SPAs, no DB lookup per request

Q: How should CORS origins be configured?
A: We'll go with option 1, aka the configurable allowlist via envvar but after the heavy dev times we'll probably switch to option 3 because the goal is to provide the app "as is" in docker e.g. with nginx as reverse proxy

Q: Should this issue include a logout endpoint that revokes the refresh token?
A: Yes + revoke-all — POST /auth/logout revokes one token; DELETE /auth/sessions revokes all (most complete, adds scope)

Q: Should a user be able to have multiple concurrent sessions (e.g., phone + laptop)?
A: Yes, unlimited — each login creates a new refresh token; simple and user-friendly

Q: What password policy should be enforced at registration?
A: 8+ characters, no composition rules (NIST SP 800-63B recommended — length matters, complexity rules add friction without proven benefit)

Q: Should auth endpoints (login, register, refresh) have rate limiting for brute-force protection?
A: Yes but keep in mind we need some way of "trusted proxies".
We want all three options:
1. Only our server --> rate limiting
2. Reverse Proxy with headers -> rate limiting using X-Real-IP or X-Forwarded-For or other options for non nginx
3. Reverse Proxy with rata liming -> no rate limiting on our side

Selecting this option via ENV / Config
