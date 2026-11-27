//! Pure-std file-watching loop for `cargo mires watch`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

use crate::build;
use crate::cli::BuildCliOptions;
use crate::metadata;

const POLL_INTERVAL: Duration = Duration::from_millis(300);

const SKIP_DIRS: [&str; 6] = ["target", "build", ".git", "node_modules", ".hg", ".svn"];

/// Lightweight metadata stamp used to detect source edits without hashing file bodies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileStamp {
    pub len: u64,
    pub modified: Option<SystemTime>,
}

/// Snapshot of watched `.rs` and `Cargo.toml` files mapped to their metadata stamps.
pub type Snapshot = BTreeMap<PathBuf, FileStamp>;

/// Run an initial build and then poll workspace/crate sources every 300ms for changes.
pub fn run(opts: &BuildCliOptions) -> Result<(), String> {
    let ctx = metadata::load_package_context(opts)?;
    let roots = watched_roots(&ctx.workspace_root, &ctx.package.manifest_dir);
    let ignore_dirs = vec![ctx.config.out_dir.clone(), ctx.target_directory.clone()];

    let mut last_snapshot = snapshot_files(&roots, &ignore_dirs);
    run_once(opts);
    eprintln!("watching for changes in .rs and Cargo.toml files (Ctrl-C to stop)...");

    loop {
        thread::sleep(POLL_INTERVAL);
        let current = snapshot_files(&roots, &ignore_dirs);
        if current == last_snapshot {
            continue;
        }
        let changed = diff_snapshots(&last_snapshot, &current);
        last_snapshot = current;

        if let Some(first) = changed.first() {
            let display = first
                .strip_prefix(&ctx.workspace_root)
                .unwrap_or(first)
                .display();
            if changed.len() == 1 {
                eprintln!("\nchange detected in {display}; rebuilding...");
            } else {
                eprintln!(
                    "\nchange detected in {display} (+{} more); rebuilding...",
                    changed.len() - 1
                );
            }
        } else {
            eprintln!("\nchange detected; rebuilding...");
        }
        run_once(opts);
    }
}

fn run_once(opts: &BuildCliOptions) {
    if let Err(err) = build::run_build(opts) {
        eprintln!("error: {err}");
    }
}

fn watched_roots(workspace_root: &Path, manifest_dir: &Path) -> Vec<PathBuf> {
    if manifest_dir.starts_with(workspace_root) {
        vec![workspace_root.to_path_buf()]
    } else {
        vec![workspace_root.to_path_buf(), manifest_dir.to_path_buf()]
    }
}

/// Recursively scan `roots` for `.rs` and `Cargo.toml` files, skipping build/VCS directories.
pub fn snapshot_files(roots: &[PathBuf], ignore_dirs: &[PathBuf]) -> Snapshot {
    let normalized_ignores: Vec<PathBuf> = ignore_dirs
        .iter()
        .map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .collect();

    let mut snapshot = BTreeMap::new();
    for root in roots {
        scan_dir(root, &normalized_ignores, &mut snapshot);
    }
    snapshot
}

fn scan_dir(dir: &Path, ignore_dirs: &[PathBuf], snapshot: &mut Snapshot) {
    if should_ignore_dir(dir, ignore_dirs) {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            scan_dir(&path, ignore_dirs, snapshot);
        } else if file_type.is_file() && is_watched_file(&path) {
            if let Ok(meta) = entry.metadata() {
                snapshot.insert(
                    path,
                    FileStamp {
                        len: meta.len(),
                        modified: meta.modified().ok(),
                    },
                );
            }
        }
    }
}

fn should_ignore_dir(dir: &Path, ignore_dirs: &[PathBuf]) -> bool {
    if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
        if SKIP_DIRS.contains(&name) {
            return true;
        }
    }
    if ignore_dirs.is_empty() {
        return false;
    }
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    ignore_dirs.iter().any(|ignored| ignored == &canonical)
}

fn is_watched_file(path: &Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") {
        return true;
    }
    path.extension().and_then(|e| e.to_str()) == Some("rs")
}

/// Return sorted paths that were added, modified, or removed between two snapshots.
pub fn diff_snapshots(before: &Snapshot, after: &Snapshot) -> Vec<PathBuf> {
    let mut changed = Vec::new();
    for (path, stamp) in after {
        if before.get(path) != Some(stamp) {
            changed.push(path.clone());
        }
    }
    for path in before.keys() {
        if !after.contains_key(path) {
            changed.push(path.clone());
        }
    }
    changed.sort();
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "cargo-mires-watch-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn snapshot_tracks_rs_and_cargo_toml_and_skips_ignored_dirs() {
        let root = unique_temp_dir("scan");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::create_dir_all(root.join("dist")).unwrap();

        let cargo_toml = root.join("Cargo.toml");
        let lib_rs = root.join("src/lib.rs");
        let readme = root.join("README.md");
        let target_rs = root.join("target/generated.rs");
        let dist_rs = root.join("dist/out.rs");

        fs::write(&cargo_toml, "[package]\nname = \"demo\"\n").unwrap();
        fs::write(&lib_rs, "pub fn hi() {}\n").unwrap();
        fs::write(&readme, "# ignore me\n").unwrap();
        fs::write(&target_rs, "fn skip() {}\n").unwrap();
        fs::write(&dist_rs, "fn skip_dist() {}\n").unwrap();

        let snap = snapshot_files(std::slice::from_ref(&root), &[root.join("dist")]);
        assert!(snap.contains_key(&cargo_toml));
        assert!(snap.contains_key(&lib_rs));
        assert!(!snap.contains_key(&readme));
        assert!(!snap.contains_key(&target_rs));
        assert!(!snap.contains_key(&dist_rs));

        // Modify a tracked file and add a new one
        let extra_rs = root.join("src/extra.rs");
        fs::write(&lib_rs, "pub fn hi() -> i32 { 42 }\n").unwrap();
        fs::write(&extra_rs, "pub fn extra() {}\n").unwrap();

        let snap2 = snapshot_files(std::slice::from_ref(&root), &[root.join("dist")]);
        let changed = diff_snapshots(&snap, &snap2);
        assert!(changed.contains(&lib_rs));
        assert!(changed.contains(&extra_rs));

        let _ = fs::remove_dir_all(&root);
    }
}
