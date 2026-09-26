//! Publish complete benchmark bytes without replacing another writer's artifact.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(0);

pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "artifact requires a filename")
    })?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let (temporary, mut file) = loop {
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = name.to_os_string();
        temporary_name.push(format!(".tmp-{}-{serial}", std::process::id()));
        let temporary = parent.join(temporary_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => break (temporary, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        // Same-directory hard-link publication is atomic and create-only.
        // rename() would replace a concurrently created destination.
        fs::hard_link(&temporary, path)
    })();
    drop(file);
    let cleanup = fs::remove_file(&temporary);
    result.and(cleanup)
}
