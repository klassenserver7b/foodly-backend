//! Unit tests for the auth service, moved out of `src/services/auth.rs`
//! so CodeQL does not flag the hard-coded test credentials that live in
//! `#[cfg(test)]` blocks inside production source files.
//!
//! `tests/**` is excluded from CodeQL analysis via `paths-ignore` in
//! `.github/codeql-config.yml`.

use foodly_backend::services::auth::{
    create_access_token, generate_refresh_token, hash_password, hash_token, verify_access_token,
    verify_password,
};

#[test]
fn test_hash_password_roundtrip() {
    // Set a test pepper so hash_password doesn't panic when
    // PASSWORD_PEPPER is unset in the test environment.
    unsafe {
        std::env::set_var(
            "PASSWORD_PEPPER",
            "test-pepper-0123456789abcdef0123456789abcdef",
        );
    }
    let hash = hash_password("hunter2").unwrap();
    assert!(hash.starts_with("$argon2"));
    assert!(verify_password("hunter2", &hash).unwrap());
    assert!(!verify_password("wrong", &hash).unwrap());
    unsafe {
        std::env::remove_var("PASSWORD_PEPPER");
    }
}

#[test]
fn test_generate_refresh_token() {
    let token = generate_refresh_token();
    assert_eq!(token.len(), 64);
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_hash_token() {
    let hash = hash_token("abc");
    assert_eq!(hash.len(), 64);
}

#[test]
fn test_jwt_roundtrip() {
    let secret = "0123456789abcdef0123456789abcdef0123456789abcdef";
    let token = create_access_token(42, secret, 15).unwrap();
    let user_id = verify_access_token(&token, secret).unwrap();
    assert_eq!(user_id, 42);
}

#[test]
fn test_jwt_wrong_secret_rejected() {
    let secret = "0123456789abcdef0123456789abcdef0123456789abcdef";
    let wrong = "fedcba9876543210fedcba9876543210fedcba9876543210";
    let token = create_access_token(42, secret, 15).unwrap();
    assert!(verify_access_token(&token, wrong).is_err());
}

#[test]
fn test_jwt_expired_rejected() {
    let secret = "0123456789abcdef0123456789abcdef0123456789abcdef";
    // leeway is 60s so an expired token within 60s might still pass; use -120 to be safe
    let old = create_access_token(42, secret, -120).unwrap();
    assert!(verify_access_token(&old, secret).is_err());
}
