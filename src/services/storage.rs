use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::fs;

pub async fn save_image(storage_dir: &Path, bytes: &[u8]) -> anyhow::Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let hash = hex::encode(hasher.finalize());

    let prefix1 = &hash[0..2];
    let prefix2 = &hash[2..4];

    let target_dir = storage_dir.join(prefix1).join(prefix2);
    fs::create_dir_all(&target_dir).await?;

    let file_path = target_dir.join(&hash);
    fs::write(&file_path, bytes).await?;

    Ok(hash)
}

/// Builds the on-disk path for a validated image hash.
/// The hash MUST already be validated as exactly 64 lowercase hex
/// characters; this function performs no filesystem checks.
///
/// # Panics
/// Panics if `hash` is not exactly 64 ASCII hex characters. All callers
/// must call [`validate_hash`] (or otherwise guarantee the format) first.
pub fn get_image_path(storage_dir: &Path, hash: &str) -> PathBuf {
    validate_hash(hash).expect("hash pre-validated by caller");
    let prefix1 = &hash[0..2];
    let prefix2 = &hash[2..4];
    storage_dir.join(prefix1).join(prefix2).join(hash)
}

/// Validates that a hash is exactly 64 lowercase ASCII hex characters
/// (the format produced by [`save_image`]).
/// Returns `Ok(())` if valid, or an error message string if not.
pub fn validate_hash(hash: &str) -> Result<(), String> {
    if hash.len() != 64 {
        return Err(format!("hash must be 64 characters, got {}", hash.len()));
    }
    if !hash
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err("hash must be lowercase hex characters only".to_string());
    }
    Ok(())
}

pub async fn read_image(storage_dir: &Path, hash: &str) -> std::io::Result<Vec<u8>> {
    let file_path = get_image_path(storage_dir, hash);
    fs::read(&file_path).await
}

pub async fn delete_image(storage_dir: &Path, hash: &str) -> std::io::Result<()> {
    let file_path = get_image_path(storage_dir, hash);
    fs::remove_file(&file_path).await
}
