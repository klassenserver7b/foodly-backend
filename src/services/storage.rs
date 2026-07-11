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

pub fn get_image_path(storage_dir: &Path, hash: &str) -> PathBuf {
    storage_dir.join(&hash[0..2]).join(&hash[2..4]).join(hash)
}

pub async fn read_image(storage_dir: &Path, hash: &str) -> std::io::Result<Vec<u8>> {
    let file_path = get_image_path(storage_dir, hash);
    fs::read(&file_path).await
}

pub async fn delete_image(storage_dir: &Path, hash: &str) -> std::io::Result<()> {
    let file_path = get_image_path(storage_dir, hash);
    fs::remove_file(&file_path).await
}
