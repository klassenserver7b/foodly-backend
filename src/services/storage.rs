use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::fs;

/// Returns the SHA-256 hex digest used as the storage key for `bytes`.
#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Writes `bytes` to the location derived from their digest.
pub async fn save_image(storage_dir: &Path, bytes: &[u8]) -> anyhow::Result<String> {
    let hash = hash_bytes(bytes);
    write_image(storage_dir, &hash, bytes).await?;
    Ok(hash)
}

/// Writes `bytes` to the on-disk location for `hash`.
///
/// `hash` must be the digest of `bytes` (see [`hash_bytes`]), e.g. for a
/// re-write after taking the per-hash mutation lock.
pub async fn write_image(storage_dir: &Path, hash: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let file_path = get_image_path(storage_dir, hash).map_err(anyhow::Error::msg)?;
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(&file_path, bytes).await?;
    Ok(())
}

/// Builds the on-disk path for an image hash.
///
/// Returns an error for hashes that are not 64 lowercase hex characters
/// (e.g. legacy seeded values such as `hash_1`). Callers should treat this
/// as "not found"; it must never panic, since the hash may come straight
/// from a database column.
pub fn get_image_path(storage_dir: &Path, hash: &str) -> Result<PathBuf, String> {
    validate_hash(hash)?;
    let prefix1 = &hash[0..2];
    let prefix2 = &hash[2..4];
    Ok(storage_dir.join(prefix1).join(prefix2).join(hash))
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
    let file_path = get_image_path(storage_dir, hash)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    fs::read(&file_path).await
}

pub async fn delete_image(storage_dir: &Path, hash: &str) -> std::io::Result<()> {
    let file_path = get_image_path(storage_dir, hash)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    fs::remove_file(&file_path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_image_path_rejects_legacy_seed_hashes() {
        assert!(get_image_path(Path::new("/tmp/storage"), "hash_1").is_err());
    }

    #[test]
    fn get_image_path_builds_prefixed_shard_path() {
        let hash = "ab".repeat(32);
        let path = get_image_path(Path::new("/tmp/storage"), &hash).unwrap();
        assert_eq!(path, Path::new("/tmp/storage/ab/ab").join(&hash));
    }
}
