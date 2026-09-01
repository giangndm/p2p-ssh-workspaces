use std::{
    io::ErrorKind,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use super::errors::{AppError, AppResult};

pub(crate) static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) async fn load_identity(path: &Path) -> AppResult<iroh::SecretKey> {
    match tokio::fs::read(path).await {
        Ok(bytes) => parse_identity(path, &bytes).await,
        Err(error) if error.kind() == ErrorKind::NotFound => create_identity(path).await,
        Err(error) => Err(error.into()),
    }
}

async fn parse_identity(path: &Path, bytes: &[u8]) -> AppResult<iroh::SecretKey> {
    let secret_key =
        iroh::SecretKey::try_from(bytes).map_err(|error| AppError::Iroh(Box::new(error)))?;
    restrict_identity_permissions(path).await?;
    Ok(secret_key)
}

async fn create_identity(path: &Path) -> AppResult<iroh::SecretKey> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let secret_key = iroh::SecretKey::generate();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("worker"),
        std::process::id(),
        counter
    ));
    let result = match write_identity(&temp_path, &secret_key).await {
        Ok(()) => publish_identity(&temp_path, path, secret_key).await,
        Err(error) => Err(error),
    };
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp_path).await;
    }
    result
}

async fn write_identity(temp_path: &Path, secret_key: &iroh::SecretKey) -> AppResult<()> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)
        .await?;
    restrict_file_permissions(&file).await?;
    file.write_all(&secret_key.to_bytes()).await?;
    file.sync_all().await?;
    Ok(())
}

async fn publish_identity(
    temp_path: &Path,
    path: &Path,
    secret_key: iroh::SecretKey,
) -> AppResult<iroh::SecretKey> {
    let result = match tokio::fs::hard_link(temp_path, path).await {
        Ok(()) => Ok(secret_key),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let bytes = tokio::fs::read(path).await?;
            parse_identity(path, &bytes).await
        }
        Err(error) => Err(error.into()),
    };
    let _ = tokio::fs::remove_file(temp_path).await;
    result
}

async fn restrict_identity_permissions(path: &Path) -> AppResult<()> {
    let mut permissions = tokio::fs::metadata(path).await?.permissions();
    restrict_permissions(&mut permissions);
    tokio::fs::set_permissions(path, permissions).await?;
    Ok(())
}

async fn restrict_file_permissions(file: &tokio::fs::File) -> AppResult<()> {
    let mut permissions = file.metadata().await?.permissions();
    restrict_permissions(&mut permissions);
    file.set_permissions(permissions).await?;
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(permissions: &mut std::fs::Permissions) {
    use std::os::unix::fs::PermissionsExt;

    permissions.set_mode(0o600);
}

#[cfg(not(unix))]
fn restrict_permissions(_permissions: &mut std::fs::Permissions) {}
