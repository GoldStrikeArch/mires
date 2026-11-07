use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

/// A directory under the system temporary directory, removed when dropped.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// A fresh empty directory, named after `label` so a leftover from a
    /// crashed run says which test made it.
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);

        let path =
            env::temp_dir().join(format!("mires-build-{}-{unique}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    /// The directory.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
