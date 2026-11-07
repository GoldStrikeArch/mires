//! The link-time check that every `__rt` member a chunk reaches for exists.
//!
//! # The failure this exists for
//!
//! The backend emits `__rt.<name>(...)` for every operation with no short inline
//! spelling, and `__rt` is whatever module the build named with
//! `-Cllvm-args=js-shim-module=<specifier>`. When that module is
//! `runtime/shim.js` the set is complete by construction, because
//! `scripts/make-esm-shim.mjs` derives the export clause from the object the
//! script actually installs. When it is a module the application wrote -- which
//! is what an island build does, because a shim is a global-scope script and an
//! island is a module -- nothing checks the two against each other.
//!
//! **A missing member is not a compile error. It is a `TypeError` the first time
//! that line runs**, in a browser, in whichever branch happens to reach it. Wave
//! 5 found it the way it is always found: the dashboard was the first island to
//! need `f2i` and `str_bytes`, and `demo-app/src/island-rt.mjs` still carries a
//! hand-written note saying to go and `grep` the emitted chunks for the next
//! one. That note is what this module replaces.
//!
//! # What it does
//!
//! For each published chunk: find the namespace import of the shim specifier,
//! collect every member read through the binding it introduced, and subtract the
//! module's exports. Anything left is reported, with the chunk it came from and
//! the module that was supposed to supply it, before the build succeeds.
//!
//! The binding is taken from the import statement rather than assumed to be
//! `__rt`, so a renamed or minified binding is still followed.
//!
//! # Why the export scanner refuses what it cannot read
//!
//! A check that silently sees no exports reports every member as missing, and a
//! check that silently ignores an export form it does not know reports the ones
//! declared that way. Both are worse than no check, because both are wrong in a
//! direction someone will work around. So [`exports`] handles the forms that
//! occur and returns an error naming the line for anything else, which fails the
//! build with a message about the checker rather than about the code.

use std::{collections::BTreeSet, fs, path::Path};

use crate::{BuildError, Chunk, Result};

/// The `-Cllvm-args` option naming the module `__rt` is imported from.
pub const SHIM_MODULE_ARG: &str = "js-shim-module";

/// Checks every chunk against `module`, the file backing the shim specifier.
///
/// # Errors
///
/// Returns `Err` if a file cannot be read, if `module` carries an export form
/// the scanner does not know, or if any chunk reaches for a member `module`
/// does not export.
pub fn check(chunks: &[Chunk], shared: Option<&Chunk>, specifier: &str, module: &Path) -> Result {
    let source = fs::read_to_string(module).map_err(|source| BuildError::Io {
        path: module.to_path_buf(),
        source,
    })?;
    let available = exports(&source, module)?;

    let mut missing: Vec<(String, String)> = Vec::new();
    for chunk in chunks.iter().chain(shared) {
        let text = fs::read_to_string(&chunk.path).map_err(|source| BuildError::Io {
            path: chunk.path.clone(),
            source,
        })?;
        for member in members(&text, specifier) {
            if !available.contains(&member) {
                missing.push((chunk.name.clone(), member));
            }
        }
    }

    if missing.is_empty() {
        return Ok(());
    }
    missing.sort();
    missing.dedup();
    Err(BuildError::ShimMember {
        module: module.to_path_buf(),
        specifier: specifier.to_owned(),
        missing: missing
            .into_iter()
            .map(|(chunk, member)| format!("{member} (reached from chunk `{chunk}`)"))
            .collect(),
    })
}

/// The names `source` exports.
///
/// Deliberately a scanner rather than a parser: the whole question is which
/// identifiers follow `export`, and the forms that answer it are few. Anything
/// else is refused rather than skipped -- see the module docs.
fn exports(source: &str, module: &Path) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    // `export {` and `export const {` both continue over several lines, and the
    // generated shim clause is written that way, so a brace run is gathered
    // before it is read.
    let mut pending: Option<String> = None;

    for line in source.lines() {
        if let Some(open) = &mut pending {
            open.push(' ');
            open.push_str(line);
            if line.contains('}') {
                let gathered = pending.take().unwrap_or_default();
                braced(&gathered, &mut out);
            }
            continue;
        }

        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("export ") else {
            continue;
        };
        let rest = rest.trim_start();

        // `export default ...` introduces no named binding, so there is nothing
        // for a `__rt.<name>` read to reach.
        if rest.starts_with("default") {
            continue;
        }

        if rest.starts_with('{') || rest.starts_with("const {") || rest.starts_with("let {") {
            if rest.contains('}') {
                braced(rest, &mut out);
            } else {
                pending = Some(rest.to_owned());
            }
            continue;
        }

        let after = ["async function ", "function ", "const ", "let ", "var ", "class "]
            .iter()
            .find_map(|keyword| rest.strip_prefix(keyword));
        if let Some(after) = after {
            if let Some(name) = identifier(after) {
                out.insert(name);
                continue;
            }
        }

        return Err(BuildError::ShimExportForm {
            module: module.to_path_buf(),
            line: line.trim().to_owned(),
        });
    }

    if let Some(open) = pending {
        return Err(BuildError::ShimExportForm {
            module: module.to_path_buf(),
            line: open.trim().to_owned(),
        });
    }
    Ok(out)
}

/// The names a `{ a, b as c }` clause introduces, which is `c` where a name is
/// renamed and `a` where it is not.
fn braced(text: &str, out: &mut BTreeSet<String>) {
    let Some(open) = text.find('{') else {
        return;
    };
    let Some(close) = text.rfind('}') else {
        return;
    };
    for part in text[open + 1..close].split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let name = match part.split_once(" as ") {
            Some((_, renamed)) => renamed.trim(),
            None => part,
        };
        if let Some(name) = identifier(name) {
            out.insert(name);
        }
    }
}

/// The identifier at the start of `text`, if it starts with one.
fn identifier(text: &str) -> Option<String> {
    let text = text.trim_start();
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return None;
    }
    let end = chars
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '$'))
        .map_or(text.len(), |(at, _)| at);
    Some(text[..end].to_owned())
}

/// Every member `text` reads through its namespace import of `specifier`.
///
/// The binding is read out of the import statement rather than assumed, so a
/// renamed or minified one is still followed. A chunk with no such import reads
/// no members, which is the ordinary case for a chunk that needs no helper.
fn members(text: &str, specifier: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for binding in namespace_bindings(text, specifier) {
        let needle = format!("{binding}.");
        let bytes = text.as_bytes();
        let mut at = 0;
        while let Some(found) = text[at..].find(&needle) {
            let start = at + found;
            at = start + needle.len();
            // A member read, not the tail of a longer identifier: `x.foo` is one
            // and `ax.foo` is not.
            let before_is_ident = start > 0
                && matches!(bytes[start - 1], b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'$' | b'.');
            if before_is_ident {
                continue;
            }
            if let Some(name) = identifier(&text[at..]) {
                out.insert(name);
            }
        }
    }
    out
}

/// The local bindings `import * as X from "<specifier>"` introduces.
fn namespace_bindings(text: &str, specifier: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("import * as ") else {
            continue;
        };
        let Some(binding) = identifier(rest) else {
            continue;
        };
        let Some(from) = rest.find(" from ") else {
            continue;
        };
        let tail = rest[from + " from ".len()..].trim();
        let quoted = tail.trim_start_matches(['"', '\'']);
        let named = quoted
            .find(['"', '\''])
            .map_or(quoted, |end| &quoted[..end]);
        if named == specifier {
            out.push(binding);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(source: &str) -> Vec<String> {
        exports(source, Path::new("x.mjs"))
            .expect("it reads")
            .into_iter()
            .collect()
    }

    #[test]
    fn every_export_form_the_shim_and_an_island_module_use_is_read() {
        let source = concat!(
            "export function f2i(x) {}\n",
            "export async function later() {}\n",
            "export const memo = 1;\n",
            "export let mutable = 2;\n",
            "export var older = 3;\n",
            "export class Widget {}\n",
            "export { alpha, beta as gamma };\n",
            "export default function unnamed() {}\n",
        );
        assert_eq!(
            names(source),
            [
                "Widget", "alpha", "f2i", "gamma", "later", "memo", "mutable", "older",
            ]
        );
    }

    #[test]
    fn the_generated_shim_clause_is_read_across_its_lines() {
        // `scripts/make-esm-shim.mjs` writes exactly this shape, one name per
        // line, and it is the whole export surface of the shim's module form.
        let source = "export const {\n  alloc,\n  str_bytes,\n  write_bytes,\n} = globalThis.__rt;\n";
        assert_eq!(names(source), ["alloc", "str_bytes", "write_bytes"]);
    }

    #[test]
    fn an_export_form_the_scanner_cannot_read_fails_the_build() {
        // Rather than being skipped, which would report the names it declares as
        // missing and send someone hunting in the wrong file.
        let error = exports("export * from \"./other.js\";\n", Path::new("x.mjs"))
            .expect_err("it is refused");
        assert!(error.to_string().contains("export * from"), "{error}");
    }

    #[test]
    fn a_member_is_found_through_the_binding_the_import_named() {
        // Not through the literal `__rt`: a minified or renamed binding is the
        // case a hard-coded name would miss silently.
        let text = concat!(
            "import * as q from \"mires-island-rt\";\n",
            "function go() { return q.str_bytes(\"x\").len + q.f2i(1, 0, 9); }\n",
        );
        assert_eq!(
            members(text, "mires-island-rt").into_iter().collect::<Vec<_>>(),
            ["f2i", "str_bytes"]
        );
    }

    #[test]
    fn a_read_through_a_different_module_is_not_counted() {
        let text = concat!(
            "import * as dom from \"mires-dom\";\n",
            "import * as rt from \"mires-island-rt\";\n",
            "const a = dom.insert(x); const b = rt.f2i(1, 0, 9);\n",
        );
        assert_eq!(
            members(text, "mires-island-rt").into_iter().collect::<Vec<_>>(),
            ["f2i"]
        );
    }

    #[test]
    fn a_longer_identifier_ending_in_the_binding_is_not_a_member_read() {
        let text = concat!(
            "import * as rt from \"m\";\n",
            "const chart = { art: 1 }; const v = chart.art + rt.real;\n",
        );
        assert_eq!(
            members(text, "m").into_iter().collect::<Vec<_>>(),
            ["real"]
        );
    }
}
