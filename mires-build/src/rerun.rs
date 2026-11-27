use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    PanicMode, Result, RustOptLevel, Toolchain, PANIC_ENV, REMAP_ROOT_ENV, RUST_MIR_INLINE_ENV,
    SPIKE_ROOT_ENV,
};

const WATCHED_ENV_VARS: [&str; 5] = [
    SPIKE_ROOT_ENV,
    "JSC_SPIKE_ROOT",
    PANIC_ENV,
    RUST_MIR_INLINE_ENV,
    REMAP_ROOT_ENV,
];

/// The inputs a compiled client program depends on, printed as
/// `cargo:rerun-if-changed` directives.
///
/// Emitting a single directive replaces cargo's default rule, which reruns a
/// build script whenever any file in the package changed. That is the point:
/// the server half of an app is edited far more often than the client half,
/// and a rebuild of `src/main.rs` should not spend a minute recompiling
/// JavaScript that cannot have changed.
#[derive(Debug, Clone)]
pub struct Rerun {
    paths: Vec<PathBuf>,
}

impl Rerun {
    /// The client crate's sources plus the toolchain files that decide what
    /// they compile to.
    #[must_use]
    pub fn new(client_root: &Path, toolchain: &Toolchain) -> Self {
        let client_dir = client_root.parent().unwrap_or(Path::new("."));
        // The directory itself covers a source file being added or removed,
        // which no per-file directive can.
        let mut paths = vec![client_dir.to_path_buf()];
        collect_inputs(client_dir, &mut paths);
        collect_path_modules(&mut paths);
        let patch_dir = toolchain.root().join("patches");
        let patches = collect_patches(&patch_dir);
        paths.extend([
            toolchain.dylib().to_path_buf(),
            toolchain.linker().to_path_buf(),
            toolchain.shim(),
            toolchain.script("compile_core.sh"),
            toolchain.script("sysroot-cache.sh"),
            toolchain.root().join("rust-toolchain.toml"),
            toolchain.sysroot_args(),
            patch_dir,
        ]);
        paths.extend(patches);
        let mires_std_dir = toolchain.root().join("mires-std");
        if mires_std_dir.is_dir() {
            paths.push(mires_std_dir.clone());
            let mut std_inputs = Vec::new();
            collect_inputs(&mires_std_dir, &mut std_inputs);
            std_inputs.sort();
            paths.extend(std_inputs);
        }
        // The unwind build mode's own inputs: its target spec, and the patches only it applies.
        if toolchain.panic_mode() == PanicMode::Unwind {
            let unwind_patches = toolchain.root().join("patches").join("unwind");
            let patches = collect_patches(&unwind_patches);
            paths.push(toolchain.unwind_target_spec());
            paths.push(unwind_patches);
            paths.extend(patches);
        }
        Self { paths }
    }

    /// Watch only the immutable sysroot selected for this build.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the checkout cannot derive the settings' cache key.
    pub fn selected_sysroot(
        toolchain: &Toolchain,
        emit_args: &str,
        opt_level: Option<RustOptLevel>,
    ) -> Result<Self> {
        let sysroot = toolchain.sysroot_for(emit_args, opt_level)?;
        Ok(Self {
            paths: vec![
                sysroot.join(".cache-key"),
                sysroot.join(".js-args"),
                sysroot.join(".rust-opt-level"),
            ],
        })
    }

    /// Print the directives.
    pub fn emit(&self) {
        self.emit_paths();
        for var in WATCHED_ENV_VARS {
            println!("cargo:rerun-if-env-changed={var}");
        }
    }

    pub(crate) fn emit_paths(&self) {
        for path in &self.paths {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    /// The watched paths, in the order they are printed.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }
}

/// Collect every `.patch` file directly inside `dir`, sorted for stable directives.
fn collect_patches(dir: &Path) -> Vec<PathBuf> {
    let mut patches: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "patch")
        })
        .collect();
    patches.sort();
    patches
}

/// Append every file under `dir` that decides what a crate compiles to: its
/// Rust sources and its manifest.
///
/// Build directories and dotted directories are skipped. Nothing under
/// `target` is an input, and walking it on a warm checkout is the difference
/// between reading a few dozen files and reading a few hundred thousand.
///
/// An unreadable directory is skipped rather than reported. Callers use this
/// before anything else can fail, and the error worth having is the one the
/// compile itself gives.
pub(crate) fn collect_inputs(dir: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name != "target" && !name.starts_with('.') {
                collect_inputs(&path, paths);
            }
        } else if name == "Cargo.toml" || path.extension().is_some_and(|ext| ext == "rs") {
            paths.push(path);
        }
    }
}

/// Append every module the collected sources reach with `#[path = "..."]` and
/// that the walk above did not already cover, following such declarations
/// transitively.
///
/// A client crate has no Cargo target to lay its modules out, so anything
/// shared with the server half is pulled in from wherever it lives: the demo
/// app keeps its islands beside the server code and its client crate is little
/// more than a list of `#[path]` declarations over them. Those files decide
/// what the client compiles to as much as the ones in the client directory, and
/// without this an edit to one rebuilds nothing.
///
/// Only the named target is watched, never the directory holding it. A `#[path]`
/// usually points into the server's own sources, and watching that directory
/// would rebuild the client on every server edit, which is the cost this whole
/// type exists to avoid. Adding or removing such a module means editing the
/// declaration as well, and the file carrying it is watched.
fn collect_path_modules(paths: &mut Vec<PathBuf>) {
    let mut queue: Vec<PathBuf> = paths.iter().filter(|path| is_rust(path)).cloned().collect();
    let mut seen: HashSet<PathBuf> = paths.iter().filter_map(|path| canonical(path)).collect();
    while let Some(source) = queue.pop() {
        let dir = source.parent().unwrap_or(Path::new("."));
        for target in path_attributes(&source) {
            let target = dir.join(target);
            // A target that does not resolve is not watched. Naming a file that
            // is not there makes cargo rerun the build script every time, which
            // is worse than the miss it would paper over, and the textual scan
            // below can read a path out of a comment.
            let Some(real) = canonical(&target) else {
                continue;
            };
            if !seen.insert(real) {
                continue;
            }
            if target.is_dir() {
                let mut nested = Vec::new();
                collect_inputs(&target, &mut nested);
                queue.extend(nested.iter().filter(|path| is_rust(path)).cloned());
                paths.push(target);
                paths.append(&mut nested);
            } else {
                queue.push(target.clone());
                paths.push(target);
            }
        }
    }
}

/// The target of every `#[path = "..."]` attribute in a source file, in the
/// order they appear.
///
/// This is a textual scan and not a parse. A build script has no syntax tree to
/// ask, and the only thing reading one attribute too many costs is a directive
/// for a file that exists anyway, which the caller checks for. Escapes in the
/// string are not decoded, so a module path spelled with one is missed rather
/// than misread.
fn path_attributes(source: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(source) else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    for rest in text.split("#[path").skip(1) {
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('"') else {
            continue;
        };
        let Some(end) = rest.find('"') else {
            continue;
        };
        targets.push(rest[..end].to_owned());
    }
    targets
}

/// Whether a watched path is a Rust source, and so worth scanning for module
/// declarations.
fn is_rust(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "rs")
}

/// A path in the one spelling two directives for the same file share, or `None`
/// if it does not exist.
fn canonical(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::TempDir;

    #[test]
    fn watches_the_toolchain_and_the_client_directory() {
        let toolchain = Toolchain::new("/spike").unwrap();
        let rerun = Rerun::new(Path::new("/app/client/app.rs"), &toolchain);
        let paths = rerun.paths();

        assert_eq!(paths.first().unwrap(), Path::new("/app/client"));
        assert!(paths.contains(&toolchain.dylib().to_path_buf()));
        assert!(paths.contains(&toolchain.linker().to_path_buf()));
        assert!(paths.contains(&toolchain.shim()));
        assert!(paths.contains(&toolchain.script("compile_core.sh")));
        assert!(paths.contains(&toolchain.script("sysroot-cache.sh")));
        assert!(paths.contains(&toolchain.root().join("rust-toolchain.toml")));
        assert!(paths.contains(&toolchain.sysroot_args()));
    }

    #[test]
    fn watches_every_sysroot_patch_in_stable_order() {
        let spike = TempDir::new("rerun-sysroot-patches");
        fs::create_dir_all(spike.path().join("patches")).unwrap();
        fs::write(spike.path().join("patches/b.patch"), "").unwrap();
        fs::write(spike.path().join("patches/a.patch"), "").unwrap();
        fs::write(spike.path().join("patches/notes.txt"), "").unwrap();
        let toolchain = Toolchain::new(spike.path()).unwrap();

        let rerun = Rerun::new(Path::new("/app/client/app.rs"), &toolchain);
        let patches: Vec<_> = rerun
            .paths()
            .iter()
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "patch")
            })
            .cloned()
            .collect();

        assert_eq!(
            patches,
            [
                spike.path().join("patches/a.patch"),
                spike.path().join("patches/b.patch"),
            ],
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn watches_the_selected_keyed_sysroot() {
        let spike = TempDir::new("rerun-keyed-sysroot");
        fs::create_dir_all(spike.path().join("scripts")).unwrap();
        fs::write(
            spike.path().join("scripts/sysroot-cache.sh"),
            format!(
                "printf '%s/build/sysroots/selected\\n' '{}'\n",
                spike.path().display(),
            ),
        )
        .unwrap();
        let toolchain = Toolchain::new(spike.path()).unwrap();

        let rerun =
            Rerun::selected_sysroot(&toolchain, "js-opt=on", Some(RustOptLevel::SizeMin)).unwrap();
        let selected = spike.path().join("build/sysroots/selected");

        assert!(rerun.paths().contains(&selected.join(".cache-key")));
        assert!(rerun.paths().contains(&selected.join(".js-args")));
        assert!(rerun.paths().contains(&selected.join(".rust-opt-level")));
    }

    #[test]
    fn collects_nested_sources_and_manifests_and_nothing_else() {
        let dir = TempDir::new("rerun-sources");
        fs::create_dir_all(dir.path().join("nested")).unwrap();
        fs::write(dir.path().join("app.rs"), "").unwrap();
        fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        fs::write(dir.path().join("notes.md"), "").unwrap();
        fs::write(dir.path().join("nested").join("deep.rs"), "").unwrap();

        let mut paths = Vec::new();
        collect_inputs(dir.path(), &mut paths);
        paths.sort();

        assert_eq!(
            paths,
            [
                dir.path().join("Cargo.toml"),
                dir.path().join("app.rs"),
                dir.path().join("nested/deep.rs"),
            ]
        );
    }

    #[test]
    fn build_and_dotted_directories_are_not_inputs() {
        let dir = TempDir::new("rerun-skips");
        fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join("target/debug/build.rs"), "").unwrap();
        fs::write(dir.path().join(".git/hook.rs"), "").unwrap();
        fs::write(dir.path().join("lib.rs"), "").unwrap();

        let mut paths = Vec::new();
        collect_inputs(dir.path(), &mut paths);

        assert_eq!(paths, [dir.path().join("lib.rs")]);
    }

    /// Watch, canonicalized, so a directive spelled through `..` compares equal
    /// to the file it names.
    fn watched(paths: &[PathBuf]) -> HashSet<PathBuf> {
        paths.iter().filter_map(|path| canonical(path)).collect()
    }

    #[test]
    fn a_module_reached_by_path_is_watched_and_the_directory_holding_it_is_not() {
        let app = TempDir::new("rerun-path-modules");
        fs::create_dir_all(app.path().join("client")).unwrap();
        fs::create_dir_all(app.path().join("island")).unwrap();
        fs::write(
            app.path().join("client/app.rs"),
            "#[path = \"../island/counter.rs\"]\nmod counter;\n",
        )
        .unwrap();
        // Reached only through the module above, and spelled without spaces.
        fs::write(
            app.path().join("island/counter.rs"),
            "#[path=\"shared.rs\"]\nmod shared;\n",
        )
        .unwrap();
        fs::write(app.path().join("island/shared.rs"), "").unwrap();
        fs::write(app.path().join("island/other.rs"), "").unwrap();

        let client = app.path().join("client");
        let mut paths = vec![client.clone()];
        collect_inputs(&client, &mut paths);
        collect_path_modules(&mut paths);
        let watched = watched(&paths);

        assert!(watched.contains(&canonical(&app.path().join("island/counter.rs")).unwrap()));
        assert!(watched.contains(&canonical(&app.path().join("island/shared.rs")).unwrap()));
        assert!(!watched.contains(&canonical(&app.path().join("island")).unwrap()));
        assert!(!watched.contains(&canonical(&app.path().join("island/other.rs")).unwrap()));
    }

    #[test]
    fn a_path_naming_a_file_that_is_not_there_is_not_watched() {
        let app = TempDir::new("rerun-path-missing");
        fs::create_dir_all(app.path().join("client")).unwrap();
        fs::write(
            app.path().join("client/app.rs"),
            "// #[path = \"../gone/module.rs\"]\n",
        )
        .unwrap();

        let client = app.path().join("client");
        let mut paths = vec![client.clone()];
        collect_inputs(&client, &mut paths);
        collect_path_modules(&mut paths);

        assert_eq!(paths, [client.clone(), client.join("app.rs")]);
    }

    #[test]
    fn a_missing_client_directory_collects_nothing() {
        let mut paths = Vec::new();
        collect_inputs(Path::new("/mires-build/does/not/exist"), &mut paths);
        assert!(paths.is_empty());
    }

    #[test]
    fn watches_unwind_patches_and_build_affecting_env_vars() {
        let spike = TempDir::new("rerun-unwind-patches");
        fs::create_dir_all(spike.path().join("patches/unwind")).unwrap();
        fs::write(spike.path().join("patches/unwind/z.patch"), "").unwrap();
        fs::write(spike.path().join("patches/unwind/a.patch"), "").unwrap();
        let toolchain = Toolchain::new(spike.path())
            .unwrap()
            .with_panic_mode(PanicMode::Unwind);

        let rerun = Rerun::new(Path::new("/app/client/app.rs"), &toolchain);
        let unwind_patches: Vec<_> = rerun
            .paths()
            .iter()
            .filter(|path| path.starts_with(spike.path().join("patches/unwind")) && path.is_file())
            .cloned()
            .collect();
        assert_eq!(
            unwind_patches,
            [
                spike.path().join("patches/unwind/a.patch"),
                spike.path().join("patches/unwind/z.patch"),
            ]
        );
        assert!(WATCHED_ENV_VARS.contains(&RUST_MIR_INLINE_ENV));
        assert!(WATCHED_ENV_VARS.contains(&REMAP_ROOT_ENV));
    }

    #[test]
    fn watches_mires_std_sources_when_present() {
        let root = TempDir::new("rerun-mires-std");
        fs::create_dir_all(root.path().join("mires-std/src/sub")).unwrap();
        fs::write(root.path().join("mires-std/Cargo.toml"), "[package]\n").unwrap();
        fs::write(root.path().join("mires-std/src/lib.rs"), "").unwrap();
        fs::write(root.path().join("mires-std/src/sub/mod.rs"), "").unwrap();
        let toolchain = Toolchain::new(root.path()).unwrap();

        let rerun = Rerun::new(Path::new("/app/client/app.rs"), &toolchain);
        let std_paths: Vec<_> = rerun
            .paths()
            .iter()
            .filter(|path| path.starts_with(root.path().join("mires-std")))
            .cloned()
            .collect();
        assert_eq!(
            std_paths,
            [
                root.path().join("mires-std"),
                root.path().join("mires-std/Cargo.toml"),
                root.path().join("mires-std/src/lib.rs"),
                root.path().join("mires-std/src/sub/mod.rs"),
            ]
        );
    }
}
