//! Shared by the integration tests (each test file uses only part of it).
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

/// A fresh, short directory under /tmp (`sun_path` holds only 104 bytes on
/// macOS, the usual temp directory is too deep), removed on drop.
pub struct ShortDir(pub PathBuf);

impl ShortDir {
    pub fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = PathBuf::from(format!("/tmp/gcm-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    pub fn socket(&self) -> PathBuf {
        self.0.join("s.sock")
    }
}

impl Drop for ShortDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
