//! Pure-Rust runtime shim trimmer and emitter.
//!
//! Replaces `scripts/minimal-shim.mjs` with an in-process implementation over the bundled
//! `runtime/shim.js` index (`crate::plan::cost::shim()`). Supports three output formats:
//!
//! * [`RuntimeFormat::Script`] — plain script assigning `globalThis.__rt.<name> = ...`.
//! * [`RuntimeFormat::EsmModule`] — lexical `$rt_<name>` ES module with dead-feature elisions and
//!   `export { $rt_<name> as <name>, ... };`.
//! * [`RuntimeFormat::InlinePrelude`] — compact lexical `$rt_<name>` prelude with the same
//!   dead-feature elisions, ending in `const __rt = { <direct>: $rt_<direct>, ... };` so a
//!   self-contained module resolves `__rt.<name>` lexically with zero global mutation.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use rcgjs_object::artifact::{Artifact, ArtifactKind, ArtifactManifest};

use crate::plan::cost::{self, ShimIndex};

/// Output shape for [`emit_minimal_shim`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeFormat {
    /// Plain script setting members on `globalThis.__rt`.
    Script,
    /// Lexical ES module exporting `$rt_<name> as <name>` with dead-feature elisions.
    EsmModule,
    /// Lexical inline prelude ending with `const __rt = { ... };` for directly referenced members.
    InlinePrelude,
}

struct EmittedShim {
    names: Vec<String>,
    source: String,
}

/// Emits the smallest closed subset of the bundled `runtime/shim.js` needed by `direct`.
///
/// Member statements are emitted in original `runtime/shim.js` declaration order. Returns an error
/// if any directly or transitively reached member is absent from `runtime/shim.js`.
pub fn emit_minimal_shim(
    direct: &BTreeSet<String>,
    format: RuntimeFormat,
) -> Result<String, String> {
    emit_minimal_shim_from(cost::shim(), direct, format).map(|emitted| emitted.source)
}

/// Emits every member of the bundled `runtime/shim.js` as either an ES module (`module == true`)
/// or a plain script (`module == false`).
pub fn emit_full_shim(module: bool) -> String {
    emit_full_shim_from(cost::shim(), module).source
}

/// Pure-Rust equivalent of `scripts/minimal-shim.mjs --manifest <manifest.json> <out.js>`.
///
/// Verifies the artifact manifest and all referenced program files, computes the union of direct
/// runtime helpers across reachable program entries (or all helpers when `full` is set), writes
/// `<out.js>`, binds `runtime.artifact` (and ESM runtime import edges) in `<manifest.json>`, and
/// re-verifies the updated manifest on disk.
pub fn emit_runtime_from_manifest(
    manifest_path: &Path,
    out_path: &Path,
    module: bool,
    full: bool,
) -> Result<(), String> {
    let mut manifest = ArtifactManifest::read(manifest_path).map_err(|err| err.to_string())?;
    let manifest_dir = manifest_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    manifest
        .verify(manifest_dir)
        .map_err(|err| err.to_string())?;
    let directory = fs::canonicalize(manifest_dir).map_err(|err| err.to_string())?;

    let (output_identity, provider_name) =
        resolve_provider_output(&directory, out_path, &manifest)?;

    let mut resources = BTreeSet::new();
    for entry_name in manifest.entries.keys() {
        let reachable = manifest
            .reachable(entry_name)
            .map_err(|err| err.to_string())?;
        resources.extend(reachable);
    }

    let index = cost::shim();
    let canonical_sha256 = cost::runtime_source_sha256();
    let (direct, requirements) = collect_manifest_requirements(
        &manifest,
        &directory,
        &resources,
        index,
        &canonical_sha256,
        &output_identity,
        module,
    )?;

    let emitted = if full {
        emit_full_shim_from(index, module)
    } else {
        let format = if module {
            RuntimeFormat::EsmModule
        } else {
            RuntimeFormat::Script
        };
        emit_minimal_shim_from(index, &direct, format)?
    };

    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|err| format!("error creating `{}`: {err}", parent.display()))?;
    }
    fs::write(out_path, emitted.source.as_bytes())
        .map_err(|err| format!("error writing `{}`: {err}", out_path.display()))?;

    for name in &requirements {
        if let Some(artifact) = manifest.artifacts.get_mut(name) {
            if let Some(runtime) = artifact.runtime.as_mut() {
                runtime.artifact = Some(provider_name.clone());
                if let Some(mod_spec) = &runtime.module {
                    artifact
                        .imports
                        .insert(mod_spec.clone(), Some(provider_name.clone()));
                }
            }
        }
    }

    let mut runtime_artifact = Artifact::new(ArtifactKind::Runtime, emitted.source.as_bytes());
    if module {
        runtime_artifact.exports = emitted.names.into_iter().collect();
    }
    manifest.artifacts.insert(provider_name, runtime_artifact);

    manifest
        .write(manifest_path)
        .map_err(|err| format!("error writing `{}`: {err}", manifest_path.display()))?;
    ArtifactManifest::read(manifest_path)
        .and_then(|reloaded| reloaded.verify(manifest_dir))
        .map_err(|err| format!("artifact verification after runtime emission failed: {err}"))?;
    Ok(())
}

fn resolve_provider_output(
    directory: &Path,
    out_path: &Path,
    manifest: &ArtifactManifest,
) -> Result<(PathBuf, String), String> {
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|err| format!("error creating `{}`: {err}", parent.display()))?;
    }
    let output_identity = if out_path.exists() {
        fs::canonicalize(out_path).map_err(|err| err.to_string())?
    } else {
        let out_parent = out_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file_name = out_path
            .file_name()
            .ok_or_else(|| "runtime output must name a file".to_owned())?;
        fs::canonicalize(out_parent)
            .map_err(|err| err.to_string())?
            .join(file_name)
    };

    let rel = output_identity
        .strip_prefix(directory)
        .map_err(|_| "runtime output must be inside the artifact package".to_owned())?;
    let mut parts = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(part) => {
                let text = part
                    .to_str()
                    .ok_or_else(|| "runtime output path must be valid UTF-8".to_owned())?;
                if text.is_empty() || text.contains('\\') {
                    return Err("runtime output must be inside the artifact package".to_owned());
                }
                parts.push(text);
            }
            _ => return Err("runtime output must be inside the artifact package".to_owned()),
        }
    }
    if parts.is_empty() {
        return Err("runtime output must be inside the artifact package".to_owned());
    }
    let provider_name = parts.join("/");

    if manifest
        .artifacts
        .get(&provider_name)
        .is_some_and(|existing| existing.kind != ArtifactKind::Runtime)
    {
        return Err("runtime output collides with a compiler artifact".to_owned());
    }

    Ok((output_identity, provider_name))
}

fn collect_manifest_requirements(
    manifest: &ArtifactManifest,
    directory: &Path,
    resources: &BTreeSet<String>,
    index: &ShimIndex,
    canonical_sha256: &str,
    output_identity: &Path,
    module: bool,
) -> Result<(BTreeSet<String>, Vec<String>), String> {
    let mut direct = BTreeSet::new();
    let mut requirements = Vec::new();

    for name in resources {
        let Some(artifact) = manifest.artifacts.get(name) else {
            continue;
        };
        if artifact.kind != ArtifactKind::Program {
            continue;
        }
        let program_text = fs::read_to_string(directory.join(name))
            .map_err(|err| format!("{name}: error reading program artifact: {err}"))?;
        let observed = cost::runtime_members(&program_text);
        let declared: BTreeSet<String> = artifact
            .runtime
            .as_ref()
            .map(|rt| {
                rt.direct
                    .iter()
                    .chain(rt.external.iter())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if observed != declared {
            return Err(format!(
                "{name}: runtime members disagree with linker manifest"
            ));
        }
        let Some(runtime) = &artifact.runtime else {
            continue;
        };
        if runtime.source_sha256 != canonical_sha256 {
            return Err("canonical runtime source differs from compiler manifest".to_owned());
        }
        if module != runtime.module.is_some() {
            return Err(format!(
                "{name}: runtime module mode disagrees with manifest"
            ));
        }
        if module && !runtime.external.is_empty() {
            let ext: Vec<&str> = runtime.external.iter().map(String::as_str).collect();
            return Err(format!(
                "{name}: canonical runtime cannot supply external host members ({}); \
                 use an application provider with shim(false)",
                ext.join(", ")
            ));
        }
        if runtime
            .external
            .iter()
            .any(|member| index.members.contains_key(member))
        {
            return Err(format!(
                "{name}: canonical runtime helper declared as external"
            ));
        }
        if let Some(mod_spec) = &runtime.module {
            if !mod_spec.starts_with("./")
                || lexical_join(directory, Path::new(mod_spec)) != output_identity
            {
                return Err(format!(
                    "{name}: output does not supply declared runtime {mod_spec}"
                ));
            }
        }
        let closed = index.closure(&runtime.direct);
        let missing: Vec<&str> = closed
            .iter()
            .filter(|member| !index.members.contains_key(member.as_str()))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "the program reaches missing runtime members: {}",
                missing.join(", ")
            ));
        }
        if closed != runtime.closure {
            return Err(format!("{name}: runtime closure differs from manifest"));
        }
        direct.extend(runtime.direct.iter().cloned());
        requirements.push(name.clone());
    }

    Ok((direct, requirements))
}

fn lexical_join(base: &Path, relative: &Path) -> PathBuf {
    let mut normalized = base.to_path_buf();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir | Component::Prefix(_) => {
                normalized = PathBuf::from(component.as_os_str());
            }
        }
    }
    normalized
}

fn emit_minimal_shim_from(
    index: &ShimIndex,
    direct: &BTreeSet<String>,
    format: RuntimeFormat,
) -> Result<EmittedShim, String> {
    if format == RuntimeFormat::InlinePrelude && direct.is_empty() {
        return Ok(EmittedShim {
            names: Vec::new(),
            source: String::new(),
        });
    }

    let needed = index.closure(direct);
    let missing: Vec<&str> = needed
        .iter()
        .filter(|name| !index.members.contains_key(name.as_str()))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "the program reaches missing runtime members: {}",
            missing.join(", ")
        ));
    }

    let names: Vec<String> = index
        .order
        .iter()
        .filter(|name| needed.contains(*name))
        .cloned()
        .collect();
    let source = render_shim(index, &names, &needed, direct, format);
    Ok(EmittedShim { names, source })
}

fn emit_full_shim_from(index: &ShimIndex, module: bool) -> EmittedShim {
    let names = index.order.clone();
    let needed: BTreeSet<String> = names.iter().cloned().collect();
    let format = if module {
        RuntimeFormat::EsmModule
    } else {
        RuntimeFormat::Script
    };
    let source = render_shim(index, &names, &needed, &needed, format);
    EmittedShim { names, source }
}

fn render_shim(
    index: &ShimIndex,
    names: &[String],
    needed: &BTreeSet<String>,
    direct: &BTreeSet<String>,
    format: RuntimeFormat,
) -> String {
    if format == RuntimeFormat::Script {
        let mut lines = Vec::with_capacity(names.len() + 3);
        lines
            .push("// The subset of runtime/shim.js this program reaches. Generated; do not edit.");
        lines.push("globalThis.__rt = globalThis.__rt || {};");
        for name in names {
            lines.push(index.members[name].as_str());
        }
        lines.push("");
        return lines.join("\n");
    }

    let elide_rc = !needed.contains("retype_rc");
    let elide_boxes = !needed.contains("box")
        && !needed.contains("tail_ref")
        && (!needed.contains("tail_slice") || elide_rc);
    let elide_byte_views = !needed.contains("_bv_enable");
    let elide_element_views = !needed.contains("_element_view");
    let elide_flatten_views = !needed.contains("_flatten_view");
    let elide_blocks = !needed.contains("_register");

    let mut statements = Vec::with_capacity(names.len());
    for name in names {
        let transformed = match name.as_str() {
            "_boxes" if elide_boxes => "const $rt__boxes = null;".to_owned(),
            "_bv" if elide_byte_views => "const $rt__bv = null;".to_owned(),
            "_element_view_of" if elide_element_views => {
                "const $rt__element_view_of = null;".to_owned()
            }
            "_flatten_view_of" if elide_flatten_views => {
                "const $rt__flatten_view_of = null;".to_owned()
            }
            "tail_slice" if elide_rc => {
                "let $rt_tail_slice = (record, key, len) => ({ $b: record[key], $o: 0, $l: len });"
                    .to_owned()
            }
            "tail_slice_raw" if elide_rc => {
                "let $rt_tail_slice_raw = (p, key) => ({ $b: p.$p.$b[p.$p.$o][key], $o: 0, $l: p.$m });"
                    .to_owned()
            }
            "_blocks" if elide_blocks => "const $rt__blocks = null;".to_owned(),
            "_blk_get" if elide_blocks => "let $rt__blk_get = () => undefined;".to_owned(),
            _ => {
                let mut stmt = index.members[name].clone();
                if elide_boxes && name == "_put" {
                    stmt = strip_put_boxes_block(&stmt);
                }
                stmt = rewrite_leading_assignment(&stmt);
                rewrite_rt_refs(&stmt)
            }
        };
        statements.push(transformed);
    }

    let raw_body = statements.join("\n");
    let (body, has_err_helper) = dedup_err_prefix(&raw_body);

    let mut output = Vec::new();
    if format == RuntimeFormat::EsmModule {
        output.push(
            "// The subset of runtime/shim.js this program reaches. Generated; do not edit."
                .to_owned(),
        );
    }
    if has_err_helper {
        output.push("const $rt_err = (m) => new Error(\"rustc_codegen_js: \" + m);".to_owned());
    }
    if !body.is_empty() {
        output.push(body);
    }

    match format {
        RuntimeFormat::EsmModule => {
            if names.is_empty() {
                output.push("export {};".to_owned());
            } else {
                let exports: Vec<String> = names
                    .iter()
                    .map(|name| format!("  $rt_{name} as {name},"))
                    .collect();
                output.push(format!("export {{\n{}\n}};", exports.join("\n")));
            }
            output.push(String::new());
        }
        RuntimeFormat::InlinePrelude => {
            let bindings: Vec<String> = names
                .iter()
                .filter(|name| direct.contains(*name))
                .map(|name| format!("{name}: $rt_{name}"))
                .collect();
            output.push(format!("const __rt = {{ {} }};", bindings.join(", ")));
            output.push(String::new());
        }
        RuntimeFormat::Script => unreachable!(),
    }

    output.join("\n")
}

/// Replaces a leading `globalThis.__rt.<ident>\s*=` with `let $rt_<ident> =`.
fn rewrite_leading_assignment(stmt: &str) -> String {
    let Some(rest) = stmt.strip_prefix("globalThis.__rt.") else {
        return stmt.to_owned();
    };
    let id_len = ident_prefix_len(rest);
    if id_len == 0 {
        return stmt.to_owned();
    }
    let name = &rest[..id_len];
    let after_name = &rest[id_len..];
    let trimmed = after_name.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let Some(rhs) = trimmed.strip_prefix('=') else {
        return stmt.to_owned();
    };
    format!("let $rt_{name} ={rhs}")
}

/// Rewrites `(globalThis.)?__rt.<ident>` references outside string literals and comments to
/// `$rt_<ident>`, ignoring references embedded in a larger identifier or property chain.
fn rewrite_rt_refs(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                let quote = bytes[i];
                let start = i;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 1;
                        i += source[i..].chars().next().map_or(1, char::len_utf8);
                        continue;
                    }
                    let ch = bytes[i];
                    i += 1;
                    if ch == quote {
                        break;
                    }
                }
                out.push_str(&source[start..i]);
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                let start = i;
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                    i += 1;
                }
                out.push_str(&source[start..i]);
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                let start = i;
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                if i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i = bytes.len();
                }
                out.push_str(&source[start..i]);
            }
            _ => {
                let rest = &source[i..];
                let prefix_len = if rest.starts_with("globalThis.__rt.") {
                    Some("globalThis.__rt.".len())
                } else if rest.starts_with("__rt.") {
                    Some("__rt.".len())
                } else {
                    None
                };
                if let Some(plen) = prefix_len {
                    let after = &rest[plen..];
                    let id_len = ident_prefix_len(after);
                    if id_len > 0 {
                        let blocked = i > 0 && is_boundary_blocker(bytes[i - 1]);
                        if blocked {
                            out.push_str(&rest[..plen + id_len]);
                        } else {
                            out.push_str("$rt_");
                            out.push_str(&after[..id_len]);
                        }
                        i += plen + id_len;
                        continue;
                    }
                }
                let ch = rest.chars().next().expect("i < bytes.len()");
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// Factors repeated `new Error(<ws>"rustc_codegen_js: ...)` prefixes into `$rt_err(<ws>"...)`
/// when at least two sites appear in `body`.
fn dedup_err_prefix(body: &str) -> (String, bool) {
    const NEEDLE: &str = "new Error(";
    const PREFIX: &str = "\"rustc_codegen_js: ";
    let mut matches = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = body[cursor..].find(NEEDLE) {
        let start = cursor + rel;
        let after_open = start + NEEDLE.len();
        let ws_len = body[after_open..]
            .bytes()
            .take_while(u8::is_ascii_whitespace)
            .count();
        let after_ws = after_open + ws_len;
        if body[after_ws..].starts_with(PREFIX) {
            let end = after_ws + PREFIX.len();
            matches.push((start, after_open, after_ws, end));
            cursor = end;
        } else {
            cursor = after_open;
        }
    }
    if matches.len() < 2 {
        return (body.to_owned(), false);
    }
    let mut out = String::with_capacity(body.len());
    let mut last = 0;
    for (start, after_open, after_ws, end) in matches {
        out.push_str(&body[last..start]);
        out.push_str("$rt_err(");
        out.push_str(&body[after_open..after_ws]);
        out.push('"');
        last = end;
    }
    out.push_str(&body[last..]);
    (out, true)
}

/// Removes `_put`'s `_boxes` registration branch when `_boxes` is elided.
///
/// Mirrors `scripts/minimal-shim.mjs`'s `_put` elision regex.
fn strip_put_boxes_block(stmt: &str) -> String {
    let Some((start, end)) = find_put_boxes_block(stmt) else {
        return stmt.to_owned();
    };
    let mut out = String::with_capacity(stmt.len() - (end - start));
    out.push_str(&stmt[..start]);
    out.push_str(&stmt[end..]);
    out
}

fn find_put_boxes_block(stmt: &str) -> Option<(usize, usize)> {
    let bytes = stmt.as_bytes();
    let mut search = 0;
    while let Some(rel) = stmt[search..].find('\n') {
        let nl = search + rel;
        search = nl + 1;
        let match_start = if nl > 0 && bytes[nl - 1] == b'\r' {
            nl - 1
        } else {
            nl
        };
        let mut p = nl + 1;
        let header_ok = consume_ws_token(stmt, &mut p, "if")
            && consume_ws_token(stmt, &mut p, "(")
            && consume_ws_token(stmt, &mut p, "value")
            && consume_ws_token(stmt, &mut p, "!==")
            && consume_ws_token(stmt, &mut p, "null")
            && consume_ws_token(stmt, &mut p, "&&")
            && consume_ws_token(stmt, &mut p, "typeof value")
            && consume_ws_token(stmt, &mut p, "===")
            && consume_ws_token(stmt, &mut p, "\"object\"")
            && consume_ws_token(stmt, &mut p, "&&");
        if !header_ok {
            continue;
        }
        let Some(has_rel) = stmt[p..].find("_boxes.has(value)") else {
            continue;
        };
        p += has_rel + "_boxes.has(value)".len();
        if !consume_ws_token(stmt, &mut p, ")") || !consume_ws_token(stmt, &mut p, "{") {
            continue;
        }
        let Some(set_rel) = stmt[p..].find("_boxes.set(") else {
            continue;
        };
        if stmt[p..p + set_rel].contains('}') {
            continue;
        }
        p += set_rel + "_boxes.set(".len();
        let Some(semi_rel) = stmt[p..].find(';') else {
            continue;
        };
        if semi_rel == 0 || bytes[p + semi_rel - 1] != b')' {
            continue;
        }
        p += semi_rel + 1;
        if !consume_ws_token(stmt, &mut p, "}") {
            continue;
        }
        return Some((match_start, p));
    }
    None
}

fn consume_ws_token(source: &str, cursor: &mut usize, token: &str) -> bool {
    let bytes = source.as_bytes();
    while *cursor < bytes.len() && bytes[*cursor].is_ascii_whitespace() {
        *cursor += 1;
    }
    if source[*cursor..].starts_with(token) {
        *cursor += token.len();
        true
    } else {
        false
    }
}

fn ident_prefix_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    let Some(&first) = bytes.first() else {
        return 0;
    };
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b'$') {
        return 0;
    }
    1 + bytes[1..]
        .iter()
        .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
        .count()
}

const fn is_boundary_blocker(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'.')
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    use rcgjs_object::artifact::{Entry, EntryRole, RuntimeRequirement, ARTIFACT_SCHEMA};
    use rcgjs_object::item::FORMAT_VERSION;
    use rcgjs_object::jir::JIR_SCHEMA;

    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "jsc-link-runtime-emit-{tag}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp test dir");
        dir
    }

    #[test]
    fn script_mode_emits_transitive_closure_in_declaration_order() {
        let out = emit_minimal_shim(
            &set(&["js_catch_unwind", "js_panic_error"]),
            RuntimeFormat::Script,
        )
        .expect("emit script");
        assert!(out.starts_with(
            "// The subset of runtime/shim.js this program reaches. Generated; do not edit.\n\
             globalThis.__rt = globalThis.__rt || {};\n"
        ));
        assert!(out.ends_with('\n'));
        // js_catch_unwind transitively pulls is_abort -> _abort_errors; js_panic_error does not
        // pull abort_error (so panics are caught while compiler aborts propagate).
        let pos_abort_errors = out
            .find("globalThis.__rt._abort_errors =")
            .expect("_abort_errors");
        let pos_is_abort = out.find("globalThis.__rt.is_abort =").expect("is_abort");
        let pos_panic = out
            .find("globalThis.__rt.js_panic_error =")
            .expect("js_panic_error");
        let pos_catch = out
            .find("globalThis.__rt.js_catch_unwind =")
            .expect("js_catch_unwind");
        assert!(pos_abort_errors < pos_is_abort);
        assert!(pos_is_abort < pos_panic);
        assert!(pos_panic < pos_catch);
        assert!(!out.contains("globalThis.__rt.abort_error ="));
        assert!(!out.contains("globalThis.__rt.js_abort ="));
    }

    #[test]
    fn esm_module_mode_applies_lexical_transform_and_elisions() {
        let out = emit_minimal_shim(
            &set(&["_put", "tail_slice", "slice_from_parts"]),
            RuntimeFormat::EsmModule,
        )
        .expect("emit esm");
        // No global mutation of __rt.
        assert!(!out.contains("globalThis.__rt."));
        // Dead-feature elisions kick in when triggers (box/retype_rc/_bv_enable/_element_view/_register) are absent.
        assert!(out.contains("const $rt__boxes = null;"), "{out}");
        assert!(out.contains("const $rt__bv = null;"), "{out}");
        assert!(out.contains("const $rt__element_view_of = null;"), "{out}");
        assert!(!out.contains("$rt__flatten_view"), "{out}");
        assert!(out.contains("const $rt__blocks = null;"), "{out}");
        assert!(out.contains("let $rt__blk_get = () => undefined;"), "{out}");
        assert!(
            out.contains(
                "let $rt_tail_slice = (record, key, len) => ({ $b: record[key], $o: 0, $l: len });"
            ),
            "{out}"
        );
        // `_put`'s `_boxes.set` branch was stripped.
        assert!(!out.contains("$rt__boxes.set("), "{out}");
        // Repeated `new Error("rustc_codegen_js: ...)` sites factored into `$rt_err`.
        assert!(
            out.contains("const $rt_err = (m) => new Error(\"rustc_codegen_js: \" + m);"),
            "{out}"
        );
        assert!(out.contains("$rt_err("), "{out}");
        assert!(out.contains("export {\n"), "{out}");
        assert!(out.contains("  $rt_tail_slice as tail_slice,\n"), "{out}");
        assert!(
            out.contains("  $rt_slice_from_parts as slice_from_parts,\n"),
            "{out}"
        );

        // Empty ESM module emits `export {};`.
        let empty =
            emit_minimal_shim(&BTreeSet::new(), RuntimeFormat::EsmModule).expect("empty esm");
        assert!(empty.ends_with("export {};\n"), "{empty}");
    }

    #[test]
    fn esm_module_mode_keeps_features_when_triggers_are_reachable() {
        let out = emit_minimal_shim(
            &set(&[
                "box",
                "_bv_enable",
                "_element_view",
                "_register",
                "retype_rc",
                "_put",
            ]),
            RuntimeFormat::EsmModule,
        )
        .expect("emit full-featured esm");
        assert!(!out.contains("const $rt__boxes = null;"));
        assert!(!out.contains("const $rt__bv = null;"));
        assert!(!out.contains("const $rt__element_view_of = null;"));
        assert!(!out.contains("const $rt__blocks = null;"));
        assert!(!out.contains("let $rt__blk_get = () => undefined;"));
        assert!(out.contains("$rt__boxes.set("));
    }

    #[test]
    fn inline_prelude_binds_only_direct_members_and_empty_is_empty() {
        let empty = emit_minimal_shim(&BTreeSet::new(), RuntimeFormat::InlinePrelude)
            .expect("empty inline");
        assert!(empty.is_empty());

        let out = emit_minimal_shim(
            &set(&["js_catch_unwind", "js_now_ms"]),
            RuntimeFormat::InlinePrelude,
        )
        .expect("inline prelude");
        assert!(out.ends_with('\n'));
        assert!(!out.contains("export "));
        assert!(!out.contains("globalThis.__rt."));
        // Transitive dependencies exist lexically as `$rt_*`...
        assert!(out.contains("let $rt__abort_errors ="), "{out}");
        assert!(out.contains("let $rt_is_abort ="), "{out}");
        assert!(out.contains("$rt_is_abort(e)"), "{out}");
        // ...while `const __rt` binds ONLY the directly requested members.
        let last_line = out.lines().last().expect("const __rt line");
        assert_eq!(
            last_line,
            "const __rt = { js_now_ms: $rt_js_now_ms, js_catch_unwind: $rt_js_catch_unwind };"
        );
    }

    #[test]
    fn rewrite_rt_refs_skips_strings_comments_and_non_boundaries() {
        let input = r#"
const a = "__rt.in_double";
const b = 'globalThis.__rt.in_single';
const c = "escaped \" __rt.still_double \" end";
const d = 'escaped \' globalThis.__rt.still_single \' end';
// __rt.in_line_comment globalThis.__rt.also_line
/* __rt.in_block_comment
   globalThis.__rt.also_block */
const e = foo__rt.nope + obj.__rt.nope2 + obj.globalThis.__rt.nope3;
const f = __rt.yes1(globalThis.__rt.yes2, (__rt.yes3));
"#;
        let rewritten = rewrite_rt_refs(input);
        assert!(rewritten.contains("\"__rt.in_double\""));
        assert!(rewritten.contains("'globalThis.__rt.in_single'"));
        assert!(rewritten.contains("\"escaped \\\" __rt.still_double \\\" end\""));
        assert!(rewritten.contains("'escaped \\' globalThis.__rt.still_single \\' end'"));
        assert!(rewritten.contains("// __rt.in_line_comment globalThis.__rt.also_line"));
        assert!(rewritten.contains("/* __rt.in_block_comment\n   globalThis.__rt.also_block */"));
        assert!(rewritten.contains("foo__rt.nope + obj.__rt.nope2 + obj.globalThis.__rt.nope3"));
        assert!(rewritten.contains("const f = $rt_yes1($rt_yes2, ($rt_yes3));"));
    }

    #[test]
    fn missing_member_reports_clear_error() {
        let err = emit_minimal_shim(
            &set(&["no_such_helper", "js_log_i32"]),
            RuntimeFormat::EsmModule,
        )
        .expect_err("must fail");
        assert!(err.contains("no_such_helper"), "{err}");
    }

    #[test]
    fn emit_full_shim_includes_new_std_helpers() {
        let script = emit_full_shim(false);
        let esm = emit_full_shim(true);
        for helper in [
            "js_eprint_str",
            "js_panic_error",
            "js_throw_error",
            "js_now_ms",
            "js_epoch_ms",
            "js_catch_unwind",
        ] {
            assert!(
                script.contains(&format!("globalThis.__rt.{helper} =")),
                "missing {helper} in script full shim"
            );
            assert!(
                esm.contains(&format!("  $rt_{helper} as {helper},")),
                "missing {helper} export in ESM full shim"
            );
        }
    }

    #[test]
    fn emit_runtime_from_manifest_round_trips_and_verifies() {
        for (module, full) in [(true, false), (false, false), (true, true)] {
            let dir = temp_dir(if module { "esm" } else { "script" });
            let program_name = "prog.js";
            let program_js = if module {
                "import * as __rt from \"./shim.js\";\nexport function main() { __rt.js_catch_unwind(() => __rt.js_panic_error(\"boom\"), 0); }\n"
            } else {
                "function main() { __rt.js_catch_unwind(() => __rt.js_panic_error(\"boom\"), 0); }\n"
            };
            fs::write(dir.join(program_name), program_js.as_bytes()).expect("write program");

            let direct = set(&["js_catch_unwind", "js_panic_error"]);
            let closure = cost::helper_closure(&direct);
            let mut program_artifact = Artifact::new(ArtifactKind::Program, program_js.as_bytes());
            program_artifact.items.insert("export:main".to_owned());
            program_artifact.exports.insert("main".to_owned());
            if module {
                program_artifact
                    .imports
                    .insert("./shim.js".to_owned(), None);
            }
            program_artifact.runtime = Some(RuntimeRequirement {
                module: module.then(|| "./shim.js".to_owned()),
                source_sha256: cost::runtime_source_sha256(),
                direct,
                closure,
                external: BTreeSet::new(),
                artifact: None,
            });

            let manifest = ArtifactManifest {
                schema: ARTIFACT_SCHEMA,
                object_format: FORMAT_VERSION,
                jir_schema: JIR_SCHEMA,
                semantic_policy: "faithful".to_owned(),
                syntax_ceiling: "latest".to_owned(),
                entries: BTreeMap::from([(
                    "program".to_owned(),
                    Entry {
                        role: EntryRole::Main,
                        program: program_name.to_owned(),
                        roots: BTreeMap::from([("main".to_owned(), "export:main".to_owned())]),
                    },
                )]),
                artifacts: BTreeMap::from([(program_name.to_owned(), program_artifact)]),
            };
            let manifest_path = dir.join("prog.js.artifacts.json");
            manifest.write(&manifest_path).expect("write manifest");

            let out_path = dir.join("shim.js");
            emit_runtime_from_manifest(&manifest_path, &out_path, module, full)
                .expect("emit_runtime_from_manifest");

            let reloaded = ArtifactManifest::read(&manifest_path).expect("read updated manifest");
            reloaded.verify(&dir).expect("verify updated manifest");

            let prog = &reloaded.artifacts[program_name];
            let rt = prog.runtime.as_ref().expect("program runtime requirement");
            assert_eq!(rt.artifact.as_deref(), Some("shim.js"));
            if module {
                assert_eq!(
                    prog.imports.get("./shim.js").and_then(Option::as_deref),
                    Some("shim.js")
                );
            }
            let rt_artifact = &reloaded.artifacts["shim.js"];
            assert_eq!(rt_artifact.kind, ArtifactKind::Runtime);
            if module {
                assert!(rt_artifact.exports.contains("js_catch_unwind"));
                assert!(rt_artifact.exports.contains("js_panic_error"));
                assert!(rt_artifact.exports.contains("is_abort"));
                assert!(rt_artifact.exports.contains("_abort_errors"));
                if full {
                    assert!(rt_artifact.exports.contains("js_now_ms"));
                } else {
                    assert!(!rt_artifact.exports.contains("js_now_ms"));
                }
            } else {
                assert!(rt_artifact.exports.is_empty());
            }

            // Mismatched module mode must fail cleanly.
            let mismatch = emit_runtime_from_manifest(&manifest_path, &out_path, !module, false);
            assert!(mismatch.is_err(), "module mode mismatch must fail");

            // Output colliding with the program artifact or escaping the package must fail cleanly.
            let collision =
                emit_runtime_from_manifest(&manifest_path, &dir.join(program_name), module, false);
            assert!(collision
                .expect_err("collision must fail")
                .contains("collides with a compiler artifact"));
            let outside = emit_runtime_from_manifest(
                &manifest_path,
                &dir.parent().expect("temp parent").join("outside-shim.js"),
                module,
                false,
            );
            assert!(outside
                .expect_err("escape must fail")
                .contains("inside the artifact package"));

            let _ = fs::remove_dir_all(&dir);
        }
    }
}
