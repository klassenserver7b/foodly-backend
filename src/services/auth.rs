//! Authentication service module.
//!
//! Contains all business logic for user registration, login, token
//! refresh, logout, and session management using JWT access tokens
//! and opaque refresh tokens stored as SHA-256 hashes.

use std::sync::OnceLock;

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// JWT claims for the access token.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub iat: usize,
}

/// Resolved JWT signing secret, loaded once and cached for the process
/// lifetime.
static JWT_SECRET: OnceLock<String> = OnceLock::new();

/// Returns the JWT signing secret, failing fast if it is unset or shorter
/// than 32 characters. HS256 with a weak secret allows token forgery, so
/// this refuses to run instead of silently degrading security.
pub fn jwt_secret() -> &'static str {
    JWT_SECRET.get_or_init(|| {
        let secret = std::env::var("JWT_SECRET").unwrap_or_default();
        if secret.len() < 32 {
            panic!(
                "JWT_SECRET must be set to at least 32 characters \
                 (generate one with `openssl rand -hex 32`)"
            );
        }
        secret
    })
}

/// Server-wide password pepper, loaded once and cached for the process
/// lifetime. Unlike the salt (which is per-password and stored in the
/// hash), the pepper is a secret kept outside the database so an
/// attacker with only a DB dump cannot brute-force passwords.
static PASSWORD_PEPPER: OnceLock<String> = OnceLock::new();

/// Returns the server-wide password pepper from `PASSWORD_PEPPER`.
/// Panics if unset — the pepper is mandatory for password hashing.
pub fn password_pepper() -> &'static str {
    PASSWORD_PEPPER.get_or_init(|| {
        let pepper = std::env::var("PASSWORD_PEPPER").unwrap_or_default();
        if pepper.is_empty() {
            panic!("PASSWORD_PEPPER must be set (generate one with `openssl rand -hex 32`)");
        }
        pepper
    })
}

/// Hash a password with argon2id and return the Phc string.
///
/// The password is peppered (the server-wide `PASSWORD_PEPPER` secret
/// is prepended), salted (a unique random salt per password, embedded
/// in the Phc string), then hashed with argon2id.
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let pepper = password_pepper();
    let peppered = format!("{pepper}{password}");

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();

    let hash = argon2
        .hash_password(peppered.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash failed: {e}"))?
        .to_string();

    Ok(hash)
}

/// Verify a password against a stored argon2id hash.
/// Returns Ok(true) on match, Ok(false) on mismatch.
pub fn verify_password(password: &str, hash: &str) -> anyhow::Result<bool> {
    let pepper = password_pepper();
    let peppered = format!("{pepper}{password}");

    let parsed_hash = PasswordHash::new(hash).map_err(|e| anyhow::anyhow!("parse: {e}"))?;
    match Argon2::default().verify_password(peppered.as_bytes(), &parsed_hash) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Generate a random refresh token (32 bytes, hex-encoded).
pub fn generate_refresh_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Hash a refresh token with SHA-256 for storage.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Returns the configured access-token lifetime in minutes
/// (`JWT_EXPIRY_MINUTES` env var, default 15).
pub fn jwt_expiry_minutes() -> i64 {
    std::env::var("JWT_EXPIRY_MINUTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15)
}

/// Returns the configured refresh-token lifetime in days
/// (`REFRESH_TOKEN_EXPIRY_DAYS` env var, default 30).
pub fn refresh_token_expiry_days() -> i64 {
    std::env::var("REFRESH_TOKEN_EXPIRY_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
}

/// Create a signed HS256 access token for the given user.
pub fn create_access_token(
    user_id: i64,
    secret: &str,
    expiry_minutes: i64,
) -> anyhow::Result<String> {
    let now = Utc::now();

    let claims = Claims {
        sub: user_id.to_string(),
        exp: (now + Duration::minutes(expiry_minutes)).timestamp() as usize,
        iat: now.timestamp() as usize,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| anyhow::anyhow!("sign: {e}"))
}

/// Verify a JWT access token and return the user_id from the `sub` claim.
pub fn verify_access_token(token: &str, secret: &str) -> anyhow::Result<i64> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 60;

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| anyhow::anyhow!("decode: {e}"))?;

    token_data
        .claims
        .sub
        .parse::<i64>()
        .map_err(|e| anyhow::anyhow!("sub not numeric: {e}"))
}

// Unit tests for this module live in tests/auth_service_tests.rs —
// moved out of src/ so CodeQL does not flag hardcoded test credentials
// in #[cfg(test)] blocks inside production source files.
