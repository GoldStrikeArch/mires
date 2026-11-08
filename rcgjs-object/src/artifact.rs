//! The linker-owned deployment graph. Paths are relative to the manifest, and SHA-256
//! checksums bind packaging to the exact programs, maps, and declarations the linker emitted.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Increment when the deployment contract changes incompatibly.
pub const ARTIFACT_SCHEMA: u32 = 2;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryRole {
    #[default]
    Main,
    Worker,
    Island,
    Procedure,
    Shared,
}

impl EntryRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Worker => "worker",
            Self::Island => "island",
            Self::Procedure => "procedure",
            Self::Shared => "shared",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "main" => Self::Main,
            "worker" => Self::Worker,
            "island" => Self::Island,
            "procedure" => Self::Procedure,
            "shared" => Self::Shared,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactManifest {
    pub schema: u32,
    pub object_format: u16,
    pub jir_schema: u16,
    pub semantic_policy: String,
    pub syntax_ceiling: String,
    pub entries: BTreeMap<String, Entry>,
    pub artifacts: BTreeMap<String, Artifact>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub role: EntryRole,
    pub program: String,
    /// Exported root name to stable Rust item identity (or `export:<name>` for legacy objects).
    pub roots: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    Program,
    SourceMap,
    Declarations,
    Runtime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub kind: ArtifactKind,
    pub bytes: u64,
    pub sha256: String,
    /// Compiler-generated imports. A local artifact path binds a cross-chunk edge; `None`
    /// declares an external module which the final packager must resolve and attest.
    pub imports: BTreeMap<String, Option<String>>,
    pub source_map: Option<String>,
    pub declarations: Option<String>,
    pub items: BTreeSet<String>,
    pub exports: BTreeSet<String>,
    pub runtime: Option<RuntimeRequirement>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRequirement {
    /// `None` for script output, whose runtime is supplied in the same realm by its loader.
    pub module: Option<String>,
    /// The canonical runtime source used by the compiler's helper-closure analysis.
    /// This is not the hash of an application's external runtime provider module.
    pub source_sha256: String,
    pub direct: BTreeSet<String>,
    pub closure: BTreeSet<String>,
    /// Host members supplied by the application, outside the canonical runtime.
    pub external: BTreeSet<String>,
    /// Filled by packaging after binding canonical helpers to verified provider bytes.
    /// Script loaders still have to supply any `external` members.
    pub artifact: Option<String>,
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn manifest_path(program: &Path) -> PathBuf {
    let mut path = program.as_os_str().to_os_string();
    path.push(".artifacts.json");
    PathBuf::from(path)
}

impl Artifact {
    pub fn new(kind: ArtifactKind, bytes: &[u8]) -> Self {
        Self {
            kind,
            bytes: bytes.len() as u64,
            sha256: sha256(bytes),
            imports: BTreeMap::new(),
            source_map: None,
            declarations: None,
            items: BTreeSet::new(),
            exports: BTreeSet::new(),
            runtime: None,
        }
    }
}

impl ArtifactManifest {
    pub fn read(path: &Path) -> io::Result<Self> {
        let manifest: Self = serde_json::from_slice(&fs::read(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn write(&self, path: &Path) -> io::Result<()> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        bytes.push(b'\n');
        fs::write(path, bytes)
    }

    /// Checks schema and graph structure without requiring external modules to be installed.
    pub fn validate(&self) -> io::Result<()> {
        let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
        if self.schema != ARTIFACT_SCHEMA {
            return Err(invalid(format!(
                "artifact schema {} is unsupported",
                self.schema
            )));
        }
        if self.object_format != crate::object::FORMAT_VERSION
            || self.jir_schema != crate::jir::JIR_SCHEMA
        {
            return Err(invalid(
                "artifact compiler/schema compatibility mismatch".to_owned(),
            ));
        }
        if self.semantic_policy != "faithful"
            || !matches!(self.syntax_ceiling.as_str(), "2018" | "latest")
        {
            return Err(invalid(
                "unsupported artifact semantic policy or syntax ceiling".to_owned(),
            ));
        }
        for (name, artifact) in &self.artifacts {
            if !relative_artifact_path(name) {
                return Err(invalid(format!(
                    "artifact path escapes its package: {name}"
                )));
            }
            if artifact.sha256.len() != 64
                || !artifact.sha256.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err(invalid(format!("invalid artifact checksum: {name}")));
            }
            for (edge, expected) in [
                (artifact.source_map.as_ref(), ArtifactKind::SourceMap),
                (artifact.declarations.as_ref(), ArtifactKind::Declarations),
            ] {
                if let Some(edge) = edge {
                    if self.artifacts.get(edge).map(|a| a.kind) != Some(expected) {
                        return Err(invalid(format!(
                            "{name} has an invalid {expected:?} edge: {edge}"
                        )));
                    }
                }
            }
            for target in artifact.imports.values().flatten() {
                if !matches!(
                    self.artifacts.get(target).map(|a| a.kind),
                    Some(ArtifactKind::Program | ArtifactKind::Runtime)
                ) {
                    return Err(invalid(format!(
                        "{name} imports a missing program: {target}"
                    )));
                }
            }
            if let Some(runtime) = &artifact.runtime {
                if runtime.source_sha256.len() != 64
                    || !runtime.source_sha256.bytes().all(|c| c.is_ascii_hexdigit())
                {
                    return Err(invalid(format!(
                        "{name} has an invalid runtime source checksum"
                    )));
                }
                if !runtime.direct.is_subset(&runtime.closure) {
                    return Err(invalid(format!(
                        "{name} runtime closure omits direct requirements"
                    )));
                }
                if !runtime.external.is_disjoint(&runtime.closure) {
                    return Err(invalid(format!(
                        "{name} external runtime members overlap canonical helpers"
                    )));
                }
                if let Some(module) = &runtime.module {
                    if !artifact.imports.contains_key(module) {
                        return Err(invalid(format!(
                            "{name} does not import its declared runtime: {module}"
                        )));
                    }
                }
                if let Some(provider) = &runtime.artifact {
                    if self.artifacts.get(provider).map(|artifact| artifact.kind)
                        != Some(ArtifactKind::Runtime)
                        || runtime.module.as_ref().is_some_and(|module| {
                            artifact.imports.get(module) != Some(&Some(provider.clone()))
                        })
                    {
                        return Err(invalid(format!(
                            "{name} runtime provider disagrees with its import graph"
                        )));
                    }
                }
            }
        }
        for (name, entry) in &self.entries {
            if self.artifacts.get(&entry.program).map(|a| a.kind) != Some(ArtifactKind::Program) {
                return Err(invalid(format!(
                    "entry {name} names a missing program: {}",
                    entry.program
                )));
            }
            if entry.role != EntryRole::Shared && entry.roots.is_empty() {
                return Err(invalid(format!("entry {name} has no roots")));
            }
            if entry
                .roots
                .values()
                .any(|root| !self.artifacts[&entry.program].items.contains(root))
            {
                return Err(invalid(format!(
                    "entry {name} names a root outside its program"
                )));
            }
        }
        Ok(())
    }

    /// Revalidates compiler-owned bytes before packaging. This does not attest external
    /// imports, bundled output, or browser behavior; those remain packaging obligations.
    pub fn verify(&self, directory: &Path) -> io::Result<()> {
        self.validate()?;
        let directory = fs::canonicalize(directory)?;
        for (name, artifact) in &self.artifacts {
            let path = fs::canonicalize(directory.join(name))?;
            if !path.starts_with(&directory) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("artifact symlink escapes its package: {name}"),
                ));
            }
            let bytes = fs::read(path)?;
            if bytes.len() as u64 != artifact.bytes || sha256(&bytes) != artifact.sha256 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("artifact bytes disagree with linker manifest: {name}"),
                ));
            }
        }
        Ok(())
    }

    /// Unique compiler-owned resources reachable from an entry, including debug/declaration
    /// edges. A transfer policy may omit these metadata resources when they are not deployed.
    pub fn reachable(&self, entry: &str) -> io::Result<BTreeSet<String>> {
        self.validate()?;
        let entry = self
            .entries
            .get(entry)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown entry"))?;
        let mut pending = vec![entry.program.clone()];
        let mut reachable = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !reachable.insert(name.clone()) {
                continue;
            }
            let artifact = &self.artifacts[&name];
            pending.extend(artifact.imports.values().flatten().cloned());
            pending.extend(artifact.source_map.iter().cloned());
            pending.extend(artifact.declarations.iter().cloned());
            pending.extend(
                artifact
                    .runtime
                    .iter()
                    .filter_map(|runtime| runtime.artifact.clone()),
            );
        }
        Ok(reachable)
    }
}

fn relative_artifact_path(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\\')
        && name.split('/').all(|part| !matches!(part, "" | "." | ".."))
        && Path::new(name)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}
