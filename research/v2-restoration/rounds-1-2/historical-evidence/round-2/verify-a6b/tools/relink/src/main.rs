//! relink capture <manifest> <keepdir>          copy a manifest and its objects somewhere stable
//! relink make <kept manifest> <out manifest> <output.js> <backend args...>
//!     the kept manifest with its options re-parsed from <backend args> (the real parser) and
//!     its output moved to <output.js>.
//! relink side <manifest> <sidedir>
//!     a copy of the link the manifest describes that touches nothing of the original: every
//!     object (and, in prune mode, every file rewritten in place) is copied into <sidedir>/in,
//!     the output goes to <sidedir>/out/<same basename>, and <sidedir>/manifest.bin describes
//!     it. <sidedir>/info.txt says `program <real output>`, `stdout` or `prune`, then one line
//!     `<real path>\t<side path>` per file the link writes in place.
use rcgjs_object::manifest::{Manifest, Mode};
use rcgjs_object::opts::{parse_args, Diagnostics};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

struct Dx(Cell<u32>);
impl Diagnostics for Dx {
    fn err(&self, message: &str) {
        eprintln!("error: {message}");
        self.0.set(self.0.get() + 1);
    }
    fn warn(&self, message: &str) {
        eprintln!("warning: {message}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args[0].as_str() {
        "capture" => {
            let bytes = std::fs::read(&args[1]).unwrap();
            let mut manifest: Manifest = postcard::from_bytes(&bytes).unwrap();
            let keep = PathBuf::from(&args[2]);
            std::fs::create_dir_all(&keep).unwrap();
            for (index, object) in manifest.objects.iter_mut().enumerate() {
                let to = keep.join(format!("obj{index}.o"));
                std::fs::copy(&*object, &to).unwrap();
                *object = to;
            }
            std::fs::write(
                keep.join("manifest.bin"),
                postcard::to_stdvec(&manifest).unwrap(),
            )
            .unwrap();
        }
        "make" => {
            let bytes = std::fs::read(&args[1]).unwrap();
            let mut manifest: Manifest = postcard::from_bytes(&bytes).unwrap();
            let dx = Dx(Cell::new(0));
            manifest.opts = parse_args(&args[4..], &dx);
            if dx.0.get() > 0 {
                std::process::exit(3);
            }
            manifest.mode = Mode::Program {
                output: Some(PathBuf::from(&args[3])),
            };
            std::fs::write(&args[2], postcard::to_stdvec(&manifest).unwrap()).unwrap();
        }
        "side" => {
            let bytes = std::fs::read(&args[1]).unwrap();
            let mut manifest: Manifest = postcard::from_bytes(&bytes).unwrap();
            let side = PathBuf::from(&args[2]);
            let inputs = side.join("in");
            std::fs::create_dir_all(&inputs).unwrap();
            std::fs::create_dir_all(side.join("out")).unwrap();
            // One copy per distinct path, so a file named twice stays one file.
            let mut copies: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
            let mut copy = |path: &Path| -> PathBuf {
                if let Some(to) = copies.get(path) {
                    return to.clone();
                }
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let to = inputs.join(format!("{}-{name}", copies.len()));
                if path.exists() {
                    std::fs::copy(path, &to).unwrap();
                }
                copies.insert(path.to_path_buf(), to.clone());
                to
            };
            for object in manifest.objects.iter_mut() {
                *object = copy(object);
            }
            let mut info = String::new();
            match &mut manifest.mode {
                Mode::Program { output: Some(output) } => {
                    info.push_str(&format!("program {}\n", output.display()));
                    *output = side.join("out").join(output.file_name().unwrap());
                }
                Mode::Program { output: None } => info.push_str("stdout\n"),
                Mode::Prune { first, rest } => {
                    info.push_str("prune\n");
                    let real = first.clone();
                    *first = copy(first);
                    info.push_str(&format!("{}\t{}\n", real.display(), first.display()));
                    for path in rest.iter_mut() {
                        let real = path.clone();
                        *path = copy(path);
                        info.push_str(&format!("{}\t{}\n", real.display(), path.display()));
                    }
                }
            }
            std::fs::write(side.join("info.txt"), info).unwrap();
            std::fs::write(
                side.join("manifest.bin"),
                postcard::to_stdvec(&manifest).unwrap(),
            )
            .unwrap();
        }
        other => panic!("unknown subcommand {other}"),
    }
}
