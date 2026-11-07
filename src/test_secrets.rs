//! Hardcoded test secrets used by `#[cfg(test)]` unit tests.
//!
//! This file is excluded from CodeQL analysis via `paths-ignore` in
//! `.github/codeql-config.yml` so the hard-coded-credential queries
//! don't fire on test fixtures inside production source files.

/// Test password pepper (prepended before hashing).
pub const TEST_PEPPER: &str = "test-pepper-0123456789abcdef0123456789abcdef";

/// Test JWT secret (must be ≥ 32 chars per the `jwt_secret()` check).
pub const TEST_JWT_SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

/// Test user password.
pub const TEST_PASSWORD: &str = "hunter2";

/// Seed password for mock users created by `cargo run --bin seed`.
pub const SEED_PASSWORD: &str = "password123";

/// A wrong JWT secret to prove verification fails on mismatch.
pub const WRONG_JWT_SECRET: &str = "fedcba9876543210fedcba9876543210fedcba9876543210";

/// Test user email.
pub const TEST_EMAIL: &str = "test@example.com";

/// Test user name.
pub const TEST_NAME: &str = "Test";
