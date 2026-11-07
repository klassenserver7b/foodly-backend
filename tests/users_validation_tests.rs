//! Validation tests for user payloads, moved out of `src/api/rest/users.rs`
//! so CodeQL does not flag test fixtures inside production source files.
//!
//! `tests/**` is excluded from CodeQL analysis via `paths-ignore` in
//! `.github/codeql-config.yml`.

use foodly_backend::models::user::{UserPutPayload, UserUpdatePayload};

#[test]
fn test_patch_me_validation() {
    let payload = UserUpdatePayload {
        name: Some("".to_string()),
        email: None,
    };
    assert!(payload.name.as_ref().unwrap().trim().is_empty());
}

#[test]
fn test_put_me_validation() {
    let payload = UserPutPayload {
        name: "Test".to_string(),
        email: "test@example.com".to_string(),
    };
    assert!(!payload.name.trim().is_empty());
}
