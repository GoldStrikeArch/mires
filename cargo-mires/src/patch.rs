//! Pure-Rust unified-diff patch applier (`-p1`) for sysroot construction.
//!
//! Replaces external `patch -p1` so Mires can patch Rust's `library/` sources
//! identically on macOS, Linux, and Windows with zero external shell tools.

use std::fs;
use std::path::{Component, Path, PathBuf};

/// A single parsed file-level patch inside a unified diff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePatch {
    /// Original path after `-p1` stripping, or `None` for `/dev/null` (file creation).
    pub old_path: Option<PathBuf>,
    /// Target path after `-p1` stripping, or `None` for `/dev/null` (file deletion).
    pub new_path: Option<PathBuf>,
    pub hunks: Vec<Hunk>,
}

/// A single unified diff hunk (`@@ -old_start,old_count +new_start,new_count @@`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<HunkLine>,
    pub old_no_newline: bool,
    pub new_no_newline: bool,
}

/// One line inside a unified diff hunk body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LastLineKind {
    None,
    Context,
    Remove,
    Add,
}

/// Parse a unified diff (with `-p1` path stripping) into file patches.
///
/// Skips leading prose comments, `diff ...` headers, `index ...` lines, and
/// stray un-paired `---` lines (such as a duplicated `--- a/...` before
/// `--- a/...` + `+++ b/...`).
pub fn parse_unified_diff_p1(patch_text: &str) -> Result<Vec<FilePatch>, String> {
    let raw_lines: Vec<&str> = patch_text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();

    let mut files = Vec::new();
    let mut index = 0;

    while index < raw_lines.len() {
        if let Some(old_raw) = raw_lines[index].strip_prefix("--- ") {
            if let Some(new_raw) = raw_lines
                .get(index + 1)
                .and_then(|l| l.strip_prefix("+++ "))
            {
                let old_path = parse_header_path_p1(old_raw)?;
                let new_path = parse_header_path_p1(new_raw)?;
                if old_path.is_none() && new_path.is_none() {
                    return Err(format!(
                        "invalid patch header at line {}: both paths are /dev/null",
                        index + 1
                    ));
                }
                index += 2;

                let mut hunks = Vec::new();
                while index < raw_lines.len() {
                    let line = raw_lines[index];
                    if line.starts_with("@@ ") {
                        let (hunk, next_index) = parse_hunk(&raw_lines, index)?;
                        hunks.push(hunk);
                        index = next_index;
                    } else if is_file_boundary(&raw_lines, index) {
                        break;
                    } else if line.is_empty() {
                        index += 1;
                    } else {
                        break;
                    }
                }

                if hunks.is_empty() {
                    return Err(format!(
                        "patch header for `{}` has no hunks",
                        display_target(old_path.as_deref(), new_path.as_deref())
                    ));
                }

                files.push(FilePatch {
                    old_path,
                    new_path,
                    hunks,
                });
                continue;
            }
        }
        index += 1;
    }

    if files.is_empty() {
        return Err("no unified diff headers (`---` / `+++`) found in patch".to_owned());
    }

    Ok(files)
}

/// Parse and apply a unified diff with `-p1` rooted at `root`.
pub fn apply_patch_p1(root: &Path, patch_text: &str) -> Result<(), String> {
    let file_patches = parse_unified_diff_p1(patch_text)?;
    for file_patch in &file_patches {
        apply_file_patch(root, file_patch)?;
    }
    Ok(())
}

fn is_file_boundary(lines: &[&str], index: usize) -> bool {
    let line = lines[index];
    if line.starts_with("diff ") {
        return true;
    }
    if line.starts_with("--- ")
        && lines
            .get(index + 1)
            .is_some_and(|next| next.starts_with("+++ ") || next.starts_with("--- "))
    {
        return true;
    }
    false
}

fn parse_header_path_p1(raw: &str) -> Result<Option<PathBuf>, String> {
    // Strip optional tab-separated timestamp (`a/foo.rs\t2026-07-29 06:36:18`).
    let path_part = raw.split('\t').next().unwrap_or(raw).trim();
    if path_part == "/dev/null" {
        return Ok(None);
    }

    // Strip quotes if git quoted the path.
    let unquoted = path_part
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(path_part);

    // `-p1`: strip the first path component and any following slash(es).
    let stripped = match unquoted.find('/') {
        Some(pos) => unquoted[pos + 1..].trim_start_matches('/'),
        None => {
            return Err(format!(
                "patch path `{unquoted}` has fewer than 2 path components for `-p1`"
            ));
        }
    };

    if stripped.is_empty() {
        return Err(format!(
            "patch path `{unquoted}` is empty after `-p1` stripping"
        ));
    }

    let rel = PathBuf::from(stripped);
    validate_relative_path(&rel)?;
    Ok(Some(rel))
}

fn validate_relative_path(path: &Path) -> Result<(), String> {
    if path.is_absolute() {
        return Err(format!(
            "patch target `{}` must be a relative path",
            path.display()
        ));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "unsafe path traversal component in patch target `{}`",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

fn parse_hunk(lines: &[&str], start_index: usize) -> Result<(Hunk, usize), String> {
    let header = lines[start_index];
    let (old_start, old_count, new_start, new_count) = parse_hunk_header(header, start_index + 1)?;

    let mut hunk_lines = Vec::with_capacity(old_count.max(new_count));
    let mut old_seen = 0usize;
    let mut new_seen = 0usize;
    let mut old_no_newline = false;
    let mut new_no_newline = false;
    let mut last_kind = LastLineKind::None;
    let mut index = start_index + 1;

    while old_seen < old_count || new_seen < new_count {
        let Some(&line) = lines.get(index) else {
            return Err(format!(
                "unexpected EOF inside hunk `{header}` (expected -{old_count}/+{new_count}, got -{old_seen}/+{new_seen})"
            ));
        };

        if line.starts_with("\\ ") {
            mark_no_newline(last_kind, &mut old_no_newline, &mut new_no_newline);
            index += 1;
            continue;
        }

        if let Some(rest) = line.strip_prefix(' ') {
            hunk_lines.push(HunkLine::Context(rest.to_owned()));
            old_seen += 1;
            new_seen += 1;
            last_kind = LastLineKind::Context;
        } else if let Some(rest) = line.strip_prefix('-') {
            hunk_lines.push(HunkLine::Remove(rest.to_owned()));
            old_seen += 1;
            last_kind = LastLineKind::Remove;
        } else if let Some(rest) = line.strip_prefix('+') {
            hunk_lines.push(HunkLine::Add(rest.to_owned()));
            new_seen += 1;
            last_kind = LastLineKind::Add;
        } else if line.is_empty() {
            // Editors occasionally strip trailing space from empty context lines.
            hunk_lines.push(HunkLine::Context(String::new()));
            old_seen += 1;
            new_seen += 1;
            last_kind = LastLineKind::Context;
        } else {
            return Err(format!(
                "unexpected line inside hunk `{header}` at line {}: `{line}`",
                index + 1
            ));
        }
        index += 1;
    }

    if old_seen != old_count || new_seen != new_count {
        return Err(format!(
            "hunk line count mismatch in `{header}`: expected -{old_count}/+{new_count}, got -{old_seen}/+{new_seen}"
        ));
    }

    // Consume trailing `\ No newline at end of file` marker immediately after hunk body.
    while lines.get(index).is_some_and(|l| l.starts_with("\\ ")) {
        mark_no_newline(last_kind, &mut old_no_newline, &mut new_no_newline);
        index += 1;
    }

    Ok((
        Hunk {
            old_start,
            old_count,
            new_start,
            new_count,
            lines: hunk_lines,
            old_no_newline,
            new_no_newline,
        },
        index,
    ))
}

fn mark_no_newline(kind: LastLineKind, old_no_newline: &mut bool, new_no_newline: &mut bool) {
    match kind {
        LastLineKind::Remove => *old_no_newline = true,
        LastLineKind::Add => *new_no_newline = true,
        LastLineKind::Context => {
            *old_no_newline = true;
            *new_no_newline = true;
        }
        LastLineKind::None => {}
    }
}

fn parse_hunk_header(
    header: &str,
    line_num: usize,
) -> Result<(usize, usize, usize, usize), String> {
    let rest = header
        .strip_prefix("@@ ")
        .ok_or_else(|| format!("invalid hunk header at line {line_num}: `{header}`"))?;
    let ranges = rest
        .split(" @@")
        .next()
        .ok_or_else(|| format!("missing closing `@@` in hunk header at line {line_num}"))?;
    let mut parts = ranges.split_whitespace();
    let old_part = parts
        .next()
        .and_then(|s| s.strip_prefix('-'))
        .ok_or_else(|| {
            format!("missing old range in hunk header at line {line_num}: `{header}`")
        })?;
    let new_part = parts
        .next()
        .and_then(|s| s.strip_prefix('+'))
        .ok_or_else(|| {
            format!("missing new range in hunk header at line {line_num}: `{header}`")
        })?;

    let (old_start, old_count) = parse_range(old_part, line_num)?;
    let (new_start, new_count) = parse_range(new_part, line_num)?;
    Ok((old_start, old_count, new_start, new_count))
}

fn parse_range(spec: &str, line_num: usize) -> Result<(usize, usize), String> {
    match spec.split_once(',') {
        Some((start_s, count_s)) => {
            let start = start_s.parse::<usize>().map_err(|err| {
                format!("invalid hunk start `{start_s}` at line {line_num}: {err}")
            })?;
            let count = count_s.parse::<usize>().map_err(|err| {
                format!("invalid hunk count `{count_s}` at line {line_num}: {err}")
            })?;
            Ok((start, count))
        }
        None => {
            let start = spec
                .parse::<usize>()
                .map_err(|err| format!("invalid hunk line `{spec}` at line {line_num}: {err}"))?;
            Ok((start, 1))
        }
    }
}

fn apply_file_patch(root: &Path, patch: &FilePatch) -> Result<(), String> {
    let rel_target = patch
        .new_path
        .as_deref()
        .or(patch.old_path.as_deref())
        .ok_or_else(|| "file patch has no target path".to_owned())?;
    let full_path = root.join(rel_target);

    let (mut lines, mut trailing_newline) = if patch.old_path.is_none() {
        (Vec::new(), false)
    } else {
        let original = fs::read_to_string(&full_path).map_err(|err| {
            format!(
                "failed to read `{}` for patching: {err}",
                full_path.display()
            )
        })?;
        split_content_lines(&original)
    };

    let mut line_offset: isize = 0;
    for (hunk_idx, hunk) in patch.hunks.iter().enumerate() {
        apply_hunk(
            &mut lines,
            &mut trailing_newline,
            &mut line_offset,
            hunk,
            rel_target,
            hunk_idx + 1,
        )?;
    }

    if patch.new_path.is_none() {
        if full_path.exists() {
            fs::remove_file(&full_path)
                .map_err(|err| format!("failed to remove `{}`: {err}", full_path.display()))?;
        }
        return Ok(());
    }

    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create `{}`: {err}", parent.display()))?;
    }

    let output = join_content_lines(&lines, trailing_newline);
    fs::write(&full_path, output.as_bytes())
        .map_err(|err| format!("failed to write patched `{}`: {err}", full_path.display()))?;
    Ok(())
}

fn split_content_lines(text: &str) -> (Vec<String>, bool) {
    if text.is_empty() {
        return (Vec::new(), false);
    }
    let trailing_newline = text.ends_with('\n');
    let body = text.strip_suffix('\n').unwrap_or(text);
    let lines = body
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
        .collect();
    (lines, trailing_newline)
}

fn join_content_lines(lines: &[String], trailing_newline: bool) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.join("\n");
    if trailing_newline {
        out.push('\n');
    }
    out
}

fn apply_hunk(
    file_lines: &mut Vec<String>,
    trailing_newline: &mut bool,
    line_offset: &mut isize,
    hunk: &Hunk,
    target_path: &Path,
    hunk_number: usize,
) -> Result<(), String> {
    let old_lines: Vec<&str> = hunk
        .lines
        .iter()
        .filter_map(|line| match line {
            HunkLine::Context(s) | HunkLine::Remove(s) => Some(s.as_str()),
            HunkLine::Add(_) => None,
        })
        .collect();

    let new_lines: Vec<String> = hunk
        .lines
        .iter()
        .filter_map(|line| match line {
            HunkLine::Context(s) | HunkLine::Add(s) => Some(s.clone()),
            HunkLine::Remove(_) => None,
        })
        .collect();

    let expected_raw: isize = if hunk.old_count == 0 {
        if hunk.old_start == 0 {
            0
        } else {
            (hunk.old_start as isize) + *line_offset
        }
    } else {
        (hunk.old_start.saturating_sub(1) as isize) + *line_offset
    };
    let expected = expected_raw.clamp(0, file_lines.len() as isize) as usize;

    // 1. Exact match at expected position or nearest shifted offset.
    if let Some(pos) = find_exact_match(file_lines, &old_lines, expected) {
        let touches_eof = pos + old_lines.len() == file_lines.len();
        file_lines.splice(pos..pos + old_lines.len(), new_lines.iter().cloned());
        update_trailing_newline(trailing_newline, touches_eof, hunk, !new_lines.is_empty());
        *line_offset += (new_lines.len() as isize) - (old_lines.len() as isize);
        return Ok(());
    }

    // 2. Fallback with POSIX `patch` fuzz up to 2 context lines on leading/trailing context.
    let leading_ctx = hunk
        .lines
        .iter()
        .take_while(|l| matches!(l, HunkLine::Context(_)))
        .count();
    let trailing_ctx = hunk
        .lines
        .iter()
        .rev()
        .take_while(|l| matches!(l, HunkLine::Context(_)))
        .count();

    for fuzz in 1..=2 {
        let trim_lead = fuzz.min(leading_ctx);
        let trim_trail = fuzz.min(trailing_ctx);
        if trim_lead + trim_trail == 0 || trim_lead + trim_trail >= old_lines.len() {
            continue;
        }
        let trimmed_old = &old_lines[trim_lead..old_lines.len() - trim_trail];
        let trimmed_new = &new_lines[trim_lead..new_lines.len() - trim_trail];
        let adjusted_expected = (expected + trim_lead).min(file_lines.len());
        if let Some(pos) = find_exact_match(file_lines, trimmed_old, adjusted_expected) {
            let touches_eof = pos + trimmed_old.len() == file_lines.len();
            file_lines.splice(pos..pos + trimmed_old.len(), trimmed_new.iter().cloned());
            update_trailing_newline(trailing_newline, touches_eof, hunk, !trimmed_new.is_empty());
            *line_offset += (trimmed_new.len() as isize) - (trimmed_old.len() as isize);
            return Ok(());
        }
    }

    Err(format!(
        "hunk #{hunk_number} (@@ -{},{} +{},{} @@) failed to apply to `{}`",
        hunk.old_start,
        hunk.old_count,
        hunk.new_start,
        hunk.new_count,
        target_path.display()
    ))
}

fn update_trailing_newline(
    trailing_newline: &mut bool,
    touches_eof: bool,
    hunk: &Hunk,
    has_new_lines: bool,
) {
    if !touches_eof {
        return;
    }
    if hunk.new_no_newline {
        *trailing_newline = false;
    } else if has_new_lines {
        *trailing_newline = true;
    }
}

fn find_exact_match(file_lines: &[String], needle: &[&str], expected: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(expected.min(file_lines.len()));
    }
    if needle.len() > file_lines.len() {
        return None;
    }
    let max_pos = file_lines.len() - needle.len();
    let start = expected.min(max_pos);

    if matches_at(file_lines, needle, start) {
        return Some(start);
    }

    let max_delta = start.max(max_pos - start);
    for delta in 1..=max_delta {
        if let Some(pos) = start.checked_add(delta) {
            if pos <= max_pos && matches_at(file_lines, needle, pos) {
                return Some(pos);
            }
        }
        if let Some(pos) = start.checked_sub(delta) {
            if matches_at(file_lines, needle, pos) {
                return Some(pos);
            }
        }
    }
    None
}

fn matches_at(file_lines: &[String], needle: &[&str], pos: usize) -> bool {
    file_lines[pos..pos + needle.len()]
        .iter()
        .zip(needle.iter())
        .all(|(actual, expected)| actual.as_str() == *expected)
}

fn display_target(old_path: Option<&Path>, new_path: Option<&Path>) -> String {
    new_path
        .or(old_path)
        .map_or_else(|| "<unknown>".to_owned(), |p| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "cargo-mires-patch-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn applies_multi_file_patch_with_prose_timestamps_and_duplicate_minus_header() {
        let root = unique_temp_dir("multi");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "line1\nold_a\nline3\n").unwrap();
        fs::write(root.join("src/b.rs"), "alpha\nbeta\ngamma\n").unwrap();

        let diff = "\
Leading explanatory prose before the diff.
Another prose line.

--- a/src/a.rs\t2026-07-29 06:36:18
+++ b/src/a.rs\t2026-07-29 06:38:56
@@ -1,3 +1,4 @@
 line1
-old_a
+new_a_1
+new_a_2
 line3
--- a/src/b.rs
--- a/src/b.rs
+++ b/src/b.rs
@@ -1,3 +1,3 @@
 alpha
-beta
+BETA
 gamma
";
        apply_patch_p1(&root, diff).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("src/a.rs")).unwrap(),
            "line1\nnew_a_1\nnew_a_2\nline3\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("src/b.rs")).unwrap(),
            "alpha\nBETA\ngamma\n"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn creates_new_file_and_handles_no_newline_markers() {
        let root = unique_temp_dir("newfile");
        fs::write(root.join("no_nl.txt"), "hello\nworld").unwrap();

        let diff = "\
--- /dev/null
+++ b/nested/created.txt
@@ -0,0 +1,2 @@
+first
+second
\\ No newline at end of file
--- a/no_nl.txt
+++ b/no_nl.txt
@@ -1,2 +1,2 @@
 hello
-world
\\ No newline at end of file
+world!
";
        apply_patch_p1(&root, diff).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("nested/created.txt")).unwrap(),
            "first\nsecond"
        );
        assert_eq!(
            fs::read_to_string(root.join("no_nl.txt")).unwrap(),
            "hello\nworld!\n"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_mismatched_context_and_path_traversal() {
        let root = unique_temp_dir("reject");
        fs::write(root.join("file.rs"), "actual\n").unwrap();

        let bad_ctx = "\
--- a/file.rs
+++ b/file.rs
@@ -1,1 +1,1 @@
-expected
+replaced
";
        assert!(apply_patch_p1(&root, bad_ctx).is_err());

        let traversal = "\
--- a/../escape.rs
+++ b/../escape.rs
@@ -1,1 +1,1 @@
-a
+b
";
        assert!(parse_unified_diff_p1(traversal).is_err());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_all_real_workspace_patches_cleanly() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let patches_dir = repo_root.join("patches");
        if !patches_dir.is_dir() {
            return;
        }

        let mut patch_files = Vec::new();
        for dir in [patches_dir.clone(), patches_dir.join("unwind")] {
            if !dir.is_dir() {
                continue;
            }
            for entry in fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("patch") {
                    patch_files.push(path);
                }
            }
        }
        patch_files.sort();
        assert!(
            !patch_files.is_empty(),
            "expected real patch files under {}",
            patches_dir.display()
        );

        for path in &patch_files {
            let content = fs::read_to_string(path).unwrap();
            let parsed = parse_unified_diff_p1(&content).unwrap_or_else(|err| {
                panic!("failed to parse real patch `{}`: {err}", path.display())
            });
            assert!(
                !parsed.is_empty(),
                "real patch `{}` produced zero file patches",
                path.display()
            );
            for file_patch in &parsed {
                assert!(
                    !file_patch.hunks.is_empty(),
                    "real patch `{}` has empty hunk list",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn applies_all_real_workspace_patches_to_rust_src_if_present() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let patches_dir = repo_root.join("patches");
        let toolchain_toml = repo_root.join("rust-toolchain.toml");
        if !patches_dir.is_dir() || !toolchain_toml.is_file() {
            return;
        }

        let channel = crate::sysroot::read_pinned_channel(repo_root);
        let mut cmd = std::process::Command::new("rustc");
        cmd.arg(format!("+{channel}")).args(["--print", "sysroot"]);
        crate::sysroot::scrub_cargo_env(&mut cmd);
        let Ok(output) = cmd.output() else {
            return;
        };
        if !output.status.success() {
            return;
        }
        let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        let rust_src_lib = sysroot.join("lib/rustlib/src/rust/library");
        if !rust_src_lib.join("core").is_dir() {
            return;
        }

        // Collect all real patches in exact build_sysroot order (patches/*.patch then patches/unwind/*.patch).
        let mut patch_files = Vec::new();
        for dir in [patches_dir.clone(), patches_dir.join("unwind")] {
            if !dir.is_dir() {
                continue;
            }
            let mut dir_patches: Vec<PathBuf> = fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("patch"))
                .collect();
            dir_patches.sort();
            patch_files.extend(dir_patches);
        }

        // Copy only the touched files from rust-src into two temporary trees:
        // one patched by our pure-Rust patcher and (on Unix) one patched by `patch -p1`
        // to verify exact byte-for-byte parity.
        let temp_root = unique_temp_dir("real-rust-src");
        let ref_root = unique_temp_dir("real-rust-src-ref");
        let mut touched_files = std::collections::BTreeSet::new();
        let mut parsed_patches = Vec::new();
        for patch_path in &patch_files {
            let text = fs::read_to_string(patch_path).unwrap();
            let parsed = parse_unified_diff_p1(&text).unwrap();
            for fp in &parsed {
                if let Some(rel) = fp.old_path.as_ref().or(fp.new_path.as_ref()) {
                    touched_files.insert(rel.clone());
                }
            }
            parsed_patches.push((patch_path.clone(), text));
        }

        for rel in &touched_files {
            // `rel` starts with `library/...`
            let Ok(inside_lib) = rel.strip_prefix("library") else {
                continue;
            };
            let src = rust_src_lib.join(inside_lib);
            if src.is_file() {
                for base in [&temp_root, &ref_root] {
                    let dst = base.join(rel);
                    fs::create_dir_all(dst.parent().unwrap()).unwrap();
                    fs::copy(&src, &dst).unwrap();
                }
            }
        }

        for (patch_path, text) in &parsed_patches {
            apply_patch_p1(&temp_root, text).unwrap_or_else(|err| {
                panic!(
                    "real patch `{}` failed to apply to rust-src: {err}",
                    patch_path.display()
                )
            });
        }

        #[cfg(unix)]
        {
            use std::process::Stdio;
            let has_patch = std::process::Command::new("patch")
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if has_patch {
                for (patch_path, _) in &parsed_patches {
                    let patch_file = fs::File::open(patch_path).unwrap();
                    let status = std::process::Command::new("patch")
                        .args(["-p1", "--forward", "--silent"])
                        .current_dir(&ref_root)
                        .stdin(Stdio::from(patch_file))
                        .status()
                        .unwrap();
                    assert!(
                        status.success(),
                        "reference `patch -p1` failed on `{}`",
                        patch_path.display()
                    );
                }
                for rel in &touched_files {
                    let rust_bytes = fs::read(temp_root.join(rel)).unwrap();
                    let ref_bytes = fs::read(ref_root.join(rel)).unwrap();
                    assert_eq!(
                        rust_bytes,
                        ref_bytes,
                        "patched file `{}` differs from `patch -p1`",
                        rel.display()
                    );
                }
            }
        }

        let _ = fs::remove_dir_all(&temp_root);
        let _ = fs::remove_dir_all(&ref_root);
    }
}
