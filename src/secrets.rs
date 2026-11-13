//! Secret management: first-run bootstrap of `JWT_SECRET` and
//! `PASSWORD_PEPPER` into the dotenv file, plus the argon2id password
//! hashing that uses them.

use rand::rngs::SysRng;
use rand::{RngExt, TryRng, rng};
use std::path::{Path, PathBuf};

/// Generates a cryptographically secure hex string of `byte_len` random bytes.
#[must_use]
pub fn generate_hex(byte_len: usize) -> String {
    let mut bytes = vec![0u8; byte_len];
    if SysRng.try_fill_bytes(&mut bytes).is_err() {
        rng().fill(&mut bytes);
    }
    hex::encode(bytes)
}

/// On first run, generates `JWT_SECRET` and `PASSWORD_PEPPER` (each 64 hex
/// chars from 32 random bytes) and persists them so they survive
/// restarts. If a secret already exists and is valid (≥32 chars and
/// not the committed placeholder), it is left untouched. If the
/// secret is set but too short (<32 chars), this ERRORS instead of
/// regenerating — silently replacing an operator-supplied secret breaks
/// every hash created with it.
pub fn bootstrap_secrets() {
    bootstrap_secret("JWT_SECRET");
    bootstrap_secret("PASSWORD_PEPPER");
    // Always enforce 0600 on the dotenv file — it holds secrets.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let env_path = resolved_env_path();
        if let Ok(meta) = std::fs::metadata(&env_path) {
            let mut perms = meta.permissions();
            if perms.mode() & 0o077 != 0 {
                perms.set_mode(0o600);
                let _ = std::fs::set_permissions(&env_path, perms);
            }
        }
    }
}

/// Generates `key` if unset or empty; persists the new value and exports
/// it for this process. If the secret is set but too short (<32 chars),
/// this PANICS instead of regenerating — silently replacing an
/// operator-supplied secret breaks every password hash created with it.
pub fn bootstrap_secret(key: &str) {
    let existing = std::env::var(key).unwrap_or_default();

    // Case 1: valid (≥32 chars, not the committed placeholder) → leave untouched
    if existing.len() >= 32 && existing != "change-me-min-32-chars-random-secret" {
        return;
    }

    // Case 2: non-empty but too short or the placeholder → ERROR, don't regenerate
    if !existing.is_empty() {
        assert!(
            existing != "change-me-min-32-chars-random-secret",
            "{key} is set to the committed placeholder. \
             Set it to a real secret (≥32 chars) or remove the line \
             so a fresh one is generated."
        );
        panic!(
            "{key} is set to {} chars but must be ≥32. \
             Fix your environment or dotenv file — refusing to \
             silently replace an operator-supplied secret.",
            existing.len()
        );
    }

    // Case 3: empty or unset → generate a fresh secret and persist it
    let secret = generate_hex(32);
    let env_path = resolved_env_path();
    upsert_env_var(key, &secret, &env_path);
    // SAFETY: we are in single-threaded startup before the async
    // runtime spawns any worker threads.
    unsafe {
        std::env::set_var(key, &secret);
    }
}

/// Resolves the dotenv file path dotenvy will read (CWD first, then parents).
fn resolved_env_path() -> PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        let candidate = dir.join(".env");
        if candidate.exists() {
            return candidate;
        }
        if !dir.pop() {
            return PathBuf::from(".env");
        }
    }
}

/// Inserts or replaces `KEY=VALUE` in the file at `path`,
/// preserving all other lines. Fails loudly if the write fails.
fn upsert_env_var(key: &str, value: &str, path: &Path) {
    let mut lines: Vec<String> = Vec::new();
    if path.exists()
        && let Ok(content) = std::fs::read_to_string(path)
    {
        for line in content.lines() {
            if line.starts_with(&format!("{key}=")) {
                // skip old value; replaced below
            } else {
                lines.push(line.to_string());
            }
        }
    }
    lines.push(format!("{key}={value}"));
    match std::fs::write(path, lines.join("\n") + "\n") {
        Ok(()) => {}
        Err(e) => panic!("failed to persist {key} to {path:?}: {e}"),
    }
    // Restrict permissions: secrets must not be world-readable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
}
