# AGENTS.md

Guidance for any Agent when working in this repository.

## Git — committing & pushing

**Never commit or push on your own.** 
If you think it is time for a commit, generate a commit message and ask the user to do the commit.
Never push on your own. The user decides for its own if he wants to push.
(This isn't about deploys — pushing `main` currently deploys
nothing. The user just wants to decide when work lands on the remote.)

**Do NOT add a `Co-Authored-By` trailer** (or any AI / Gemini / Claude co-author line) to
commits in this repo.

## Checks

Always use `cargo check` to validate your changes when you think they should compile.
Use `cargo clippy` sometimes to stay close to rust guidelines.
If you ask the user to commit check `prek` first to comply with pre commit guidelines
If you changed API endpoints and their logic **start the server** and use test queries to check if they are working correctly.
If unsure present the query to the user, let him run it and evaluate the results together.

## Pre-Commit

If the user hasn't set up `prek` as a pre commit tool, ask him to do so and assist him by that.
If you add important new features that live outside the current prek checks, ask the user if they should be added.
Added them if approved.

## What this is

Always read the `README.md` and `resources/md/01_Architecture_Overview.md` to know about this project.
Consult `resources/md` for further information


### Backend architecture — conventions

- **Axum Framework**: Use `axum` for all HTTP and WebSocket routing. Route definitions and handlers should live in `src/api/`.
- **Database**: We use PostgreSQL with `sqlx`. All database interactions should utilize the `PgPool` injected via Axum's state. Prefer compile-time checked macros (`query!`, `query_as!`) for safety.
- **Error Handling**: We use `anyhow` for robust error propagation. Since `anyhow::Error` does not implement `IntoResponse` directly, use a custom wrapper (e.g., `struct AppError(anyhow::Error)`) that implements `IntoResponse`. Never panic in a handler; always bubble up the error and let the global error handler map it to an HTTP response.
- **State Management**: Use `axum::extract::State` to access shared resources like the DB pool and configuration.

## Dependencies

- **Always ask the user** before adding new crates to `Cargo.toml`.
- **Only use a crate** if it is needed and simplifies the code significantly.
- If you are unsure whether to add a crate or implement the logic directly, **stop and ask the user** for their preference.

## Testing

- **Unit Tests**: Place unit tests in the same file as the module being tested, inside a `#[cfg(test)] mod tests { ... }` block. Focus on testing domain logic and service functions.
- **Integration Tests**: Place integration tests in the `tests/` directory at the project root.
- **API Testing**: For API routes, use `tower::ServiceExt` (e.g., `app.oneshot(request)`) to bypass the network stack and test handlers directly. For database-dependent tests, ensure a dedicated test database is spun up and seeded before the test runs (or use `sqlx` test macros).
 
If you are modifing code always evaluate if new tests are needed and wirte them if so.

## Commands

```bash
cargo check          # Fast compilation check
cargo run            # Run the local development server
cargo clippy         # Run the Rust linter
cargo fmt            # Format code
docker compose up -d # Start the local dev Postgres database
```

## Formatting & Linting

We adhere to standard Rust formatting. 
- Run `cargo fmt` to format your code before committing.
- Run `cargo clippy` to catch idiomatic issues and ensure there are no warnings.

## Comments

Comments explain **why**, not **what** — well-named code already says what it
does. The main thing to avoid: a wall of comment above every function/type
restating its name and signature. Don't do that.

- Keep the non-obvious: a tricky invariant, a workaround and its reason, a
  protocol assumption, a deliberate edge-case choice. Delete comments that just
  restate the code.
- Reach for a better name before a comment. If a comment explains a variable or
  function, try renaming it first.
- **File header:** a short note on the file's role is fine (a few lines) — just
  don't let it grow into an essay.
- **Commented-out code** is OK when it earns its place (a documented alternative,
  a temporarily-disabled path) — say why it's there; otherwise delete it.
- `TODO`/`FIXME` only if actionable.

## Rustdoc

In contrary to comments, rustdoc is for **what** and **how**. It should be used to document public APIs, modules, and structs. Use `///` for documenting items and `//!` for module-level documentation.
Everything that is a widely used struct / function should have rustdoc. If you are unsure, ask the user.

## File Structure

- `src/api/`: Axum routers and handlers (HTTP/WebSocket)
- `src/db/`: Database connection setup and `.sql` migration files
- `src/models/`: Shared domain structs and database entities
- `src/services/`: Core business logic (optional, as needed for complex flows)

## Documentation
- `resources/md` contains all docs and the current state of the project.
If endpoints oder important decisions are changed, always update the docs to keep them aligned with the current codebase
