//! Publish complete benchmark bytes without replacing another writer's artifact.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(0);

pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_new_with_serial(path, bytes, &SERIAL)
}

fn write_new_with_serial(path: &Path, bytes: &[u8], serial: &AtomicU64) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "artifact requires a filename")
    })?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let (temporary, mut file) = loop {
        let serial = serial.fetch_add(1, Ordering::Relaxed);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("taskmesh-artifact-{name}-{}", std::process::id()));
            fs::create_dir(&path).expect("fresh test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn publish_before_timeout(
        path: PathBuf,
        bytes: &'static [u8],
        serial: Arc<AtomicU64>,
    ) -> io::Result<()> {
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender
                .send(write_new_with_serial(&path, bytes, &serial))
                .unwrap();
        });
        // A retry regression must fail an assertion rather than hang a test
        // process on an occupied filename or a missing parent directory.
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("artifact publisher must return promptly");
        worker.join().unwrap();
        result
    }

    #[test]
    fn publish_retries_tempfile_collisions_without_removing_foreign_bytes() {
        let directory = TestDirectory::new("tempfile-collision");
        let path = directory.0.join("raw.json");
        let temporary = |serial| {
            directory
                .0
                .join(format!("raw.json.tmp-{}-{serial}", std::process::id()))
        };
        fs::write(temporary(0), b"foreign zero").unwrap();
        fs::write(temporary(1), b"foreign one").unwrap();
        // An owner-local counter makes the collisions deterministic without
        // resetting the process-wide sequence used by concurrent publishers.
        let serial = Arc::new(AtomicU64::new(0));

        publish_before_timeout(path.clone(), b"complete artifact", serial.clone()).unwrap();

        assert_eq!(serial.load(Ordering::Relaxed), 3);
        assert_eq!(fs::read(&path).unwrap(), b"complete artifact");
        assert_eq!(fs::read(temporary(0)).unwrap(), b"foreign zero");
        assert_eq!(fs::read(temporary(1)).unwrap(), b"foreign one");
        assert!(!temporary(2).exists());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 3);
    }

    #[test]
    fn non_collision_open_failure_returns_promptly_without_retry() {
        let directory = TestDirectory::new("missing-parent");
        let path = directory.0.join("missing/raw.json");
        let serial = Arc::new(AtomicU64::new(0));
        let result = publish_before_timeout(path, b"artifact", serial.clone());

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(serial.load(Ordering::Relaxed), 1);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
    }
}
