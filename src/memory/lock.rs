use crate::{
    canonical,
    contracts::{self, LockRecord, WireDocument},
    diagnostic::{Error, Result},
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static LOCK_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn read_lock(path: impl AsRef<Path>) -> Result<LockRecord> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|e| {
        Error::new("E_IO", format!("read lock: {e}")).at(path.display().to_string())
    })?;
    let value = canonical::parse(&bytes)?;
    contracts::decode(value)
}

pub fn write_lock(path: impl AsRef<Path>, lock: &LockRecord) -> Result<()> {
    let path = path.as_ref();
    let mut normalized = lock.clone();
    normalized.normalize()?;
    let value =
        serde_json::to_value(normalized).map_err(|e| Error::new("E_CONTRACT", e.to_string()))?;
    let bytes =
        serde_json::to_vec_pretty(&value).map_err(|e| Error::new("E_JSON", e.to_string()))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| {
        Error::new("E_IO", format!("create lock directory: {e}")).at(parent.display().to_string())
    })?;
    let sequence = LOCK_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary: PathBuf = parent.join(format!(".lock-{}-{sequence}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| {
            Error::new("E_IO", format!("create lock temporary: {e}"))
                .at(temporary.display().to_string())
        })?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| {
            Error::new("E_IO", format!("write lock: {e}")).at(temporary.display().to_string())
        })?;
    drop(file);
    fs::rename(&temporary, path).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        Error::new("E_IO", format!("install lock: {e}")).at(path.display().to_string())
    })
}
