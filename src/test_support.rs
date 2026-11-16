//! Fixtures shared by the unit tests.

use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh directory under the temp dir, removed on drop.
pub struct Scratch(PathBuf);

impl Scratch {
    /// Unique per call, so tests that run in parallel can share a `name`
    /// (pinned by `tests::two_scratches_with_one_name_are_apart`).
    pub fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("waydroid-tray-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Writes `contents` to `relative`, creating its directories.
    pub fn write(&self, relative: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }
}

impl Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_scratches_with_one_name_are_apart() {
        let (a, b) = (Scratch::new("same"), Scratch::new("same"));
        assert_ne!(&*a, &*b);
        drop(a);
        assert!(b.exists());
    }
}
