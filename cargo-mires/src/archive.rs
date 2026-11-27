//! Pure-Rust `.tar.gz` archiver and extractor (USTAR + PAX extended headers).
//!
//! Zero external `tar` binary or C dependencies: uses `flate2`'s pure-Rust
//! `miniz_oxide` backend, preserves Unix executable modes, supports paths > 100
//! bytes via POSIX PAX extended headers (`'x'`), and strictly rejects path
//! traversal (`..`, absolute paths, and escaping symlinks/hardlinks).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

const BLOCK_SIZE: usize = 512;
const COPY_BUF_SIZE: usize = 64 * 1024;

/// Pack the entire contents of `src_dir` into a `.tar.gz` archive at `archive_path`.
///
/// Entries are written in deterministic sorted POSIX path order with relative
/// paths rooted inside `src_dir`. Paths > 100 bytes or non-ASCII use POSIX PAX
/// `'x'` extended headers.
pub fn create_tar_gz(src_dir: &Path, archive_path: &Path) -> Result<(), String> {
    if !src_dir.is_dir() {
        return Err(format!(
            "cannot archive `{}`: directory does not exist",
            src_dir.display()
        ));
    }
    if let Some(parent) = archive_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                format!("failed to create directory `{}`: {err}", parent.display())
            })?;
        }
    }

    let mut entries = Vec::new();
    collect_entries(src_dir, Path::new(""), &mut entries)?;
    entries.sort_by(|a, b| a.rel_posix.cmp(&b.rel_posix));

    let file = File::create(archive_path)
        .map_err(|err| format!("failed to create `{}`: {err}", archive_path.display()))?;
    let gz = GzEncoder::new(BufWriter::new(file), Compression::default());
    let mut writer = TarWriter { inner: gz };

    for entry in &entries {
        if entry.is_dir {
            let dir_path = format!("{}/", entry.rel_posix);
            writer.write_header(&dir_path, entry.mode, 0, b'5')?;
        } else {
            writer.write_file(&entry.rel_posix, &entry.full_path, entry.mode, entry.size)?;
        }
    }

    writer.finish().map_err(|err| {
        format!(
            "failed to finalize archive `{}`: {err}",
            archive_path.display()
        )
    })
}

/// Extract a `.tar.gz` archive at `archive_path` into `dest_dir`.
///
/// Rejects corrupted checksums, absolute paths, `..` traversal components, and
/// symlinks or hardlinks pointing outside `dest_dir`.
pub fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<(), String> {
    let file = File::open(archive_path)
        .map_err(|err| format!("failed to open archive `{}`: {err}", archive_path.display()))?;
    let mut reader = BufReader::new(GzDecoder::new(BufReader::new(file)));
    fs::create_dir_all(dest_dir)
        .map_err(|err| format!("failed to create `{}`: {err}", dest_dir.display()))?;

    let mut global_pax: BTreeMap<String, String> = BTreeMap::new();
    let mut next_pax: BTreeMap<String, String> = BTreeMap::new();
    let mut gnu_long_name: Option<String> = None;
    let mut gnu_long_link: Option<String> = None;
    let mut header_buf = [0u8; BLOCK_SIZE];

    loop {
        if !read_header_block(&mut reader, &mut header_buf)? {
            break;
        }
        if header_buf.iter().all(|&b| b == 0) {
            break;
        }

        verify_checksum(&header_buf)?;
        let header = parse_header(&header_buf)?;

        match header.typeflag {
            b'x' => {
                let data = read_exact_entry_bytes(&mut reader, header.size)?;
                parse_pax_records(&data, &mut next_pax)?;
                continue;
            }
            b'g' => {
                let data = read_exact_entry_bytes(&mut reader, header.size)?;
                parse_pax_records(&data, &mut global_pax)?;
                continue;
            }
            b'L' => {
                let data = read_exact_entry_bytes(&mut reader, header.size)?;
                gnu_long_name = Some(trim_nul_utf8(&data)?);
                continue;
            }
            b'K' => {
                let data = read_exact_entry_bytes(&mut reader, header.size)?;
                gnu_long_link = Some(trim_nul_utf8(&data)?);
                continue;
            }
            _ => {}
        }

        let entry_path_raw = gnu_long_name
            .take()
            .or_else(|| next_pax.remove("path"))
            .or_else(|| global_pax.get("path").cloned())
            .unwrap_or(header.name);
        let link_target_raw = gnu_long_link
            .take()
            .or_else(|| next_pax.remove("linkpath"))
            .or_else(|| global_pax.get("linkpath").cloned())
            .unwrap_or(header.linkname);
        let entry_size = next_pax
            .remove("size")
            .or_else(|| global_pax.get("size").cloned())
            .map(|s| {
                s.parse::<u64>()
                    .map_err(|err| format!("invalid PAX size `{s}`: {err}"))
            })
            .transpose()?
            .unwrap_or(header.size);
        next_pax.clear();

        extract_single_entry(
            &mut reader,
            dest_dir,
            &entry_path_raw,
            &link_target_raw,
            header.typeflag,
            header.mode,
            entry_size,
        )?;
    }

    Ok(())
}

struct WalkEntry {
    rel_posix: String,
    full_path: PathBuf,
    is_dir: bool,
    mode: u32,
    size: u64,
}

fn collect_entries(root: &Path, rel: &Path, out: &mut Vec<WalkEntry>) -> Result<(), String> {
    let dir = if rel.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    let entries = fs::read_dir(&dir)
        .map_err(|err| format!("failed to read directory `{}`: {err}", dir.display()))?;

    let mut children: Vec<PathBuf> = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("failed to iterate `{}`: {err}", dir.display()))?
        .into_iter()
        .map(|entry| entry.path())
        .collect();
    children.sort();

    for full_path in children {
        let Some(name) = full_path.file_name() else {
            continue;
        };
        let child_rel = rel.join(name);
        let meta = fs::symlink_metadata(&full_path)
            .map_err(|err| format!("failed to stat `{}`: {err}", full_path.display()))?;
        let rel_posix = path_to_posix(&child_rel)?;

        if meta.file_type().is_dir() {
            out.push(WalkEntry {
                rel_posix,
                full_path: full_path.clone(),
                is_dir: true,
                mode: dir_mode(&meta),
                size: 0,
            });
            collect_entries(root, &child_rel, out)?;
        } else if meta.file_type().is_file() {
            out.push(WalkEntry {
                rel_posix,
                full_path,
                is_dir: false,
                mode: file_mode(&meta),
                size: meta.len(),
            });
        } else if meta.file_type().is_symlink() {
            // Follow symlink to regular file/dir if it stays within `root`, else skip.
            let target_meta = fs::metadata(&full_path).map_err(|err| {
                format!(
                    "failed to follow symlink `{}` while archiving: {err}",
                    full_path.display()
                )
            })?;
            if target_meta.is_file() {
                out.push(WalkEntry {
                    rel_posix,
                    full_path,
                    is_dir: false,
                    mode: file_mode(&target_meta),
                    size: target_meta.len(),
                });
            } else if target_meta.is_dir() {
                out.push(WalkEntry {
                    rel_posix,
                    full_path: full_path.clone(),
                    is_dir: true,
                    mode: dir_mode(&target_meta),
                    size: 0,
                });
                collect_entries(root, &child_rel, out)?;
            }
        }
    }
    Ok(())
}

fn path_to_posix(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(os) => {
                let s = os
                    .to_str()
                    .ok_or_else(|| format!("non-UTF-8 path component in `{}`", path.display()))?;
                parts.push(s);
            }
            Component::CurDir => {}
            _ => {
                return Err(format!(
                    "unexpected non-relative path `{}` while archiving",
                    path.display()
                ));
            }
        }
    }
    Ok(parts.join("/"))
}

#[cfg(unix)]
fn file_mode(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o111 != 0 {
        0o755
    } else {
        0o644
    }
}

#[cfg(not(unix))]
fn file_mode(_meta: &fs::Metadata) -> u32 {
    0o644
}

fn dir_mode(_meta: &fs::Metadata) -> u32 {
    0o755
}

struct TarWriter<W: Write> {
    inner: GzEncoder<W>,
}

impl<W: Write> TarWriter<W> {
    fn write_header(
        &mut self,
        path: &str,
        mode: u32,
        size: u64,
        typeflag: u8,
    ) -> Result<(), String> {
        let needs_pax = path.len() > 100 || !path.is_ascii() || size >= (1u64 << 33);
        if needs_pax {
            let mut pax = Vec::new();
            if path.len() > 100 || !path.is_ascii() {
                pax.extend_from_slice(&format_pax_record("path", path));
            }
            if size >= (1u64 << 33) {
                pax.extend_from_slice(&format_pax_record("size", &size.to_string()));
            }
            let pax_name = pax_header_name(path);
            let pax_block = build_ustar_header(&pax_name, 0o644, pax.len() as u64, b'x');
            self.write_all_bytes(&pax_block)?;
            self.write_all_bytes(&pax)?;
            self.write_padding(pax.len() as u64)?;
        }

        let ustar_name = if path.len() <= 100 && path.is_ascii() {
            path.to_owned()
        } else {
            truncate_ascii_name(path, 100)
        };
        let header_size = if size < (1u64 << 33) { size } else { 0 };
        let header_block = build_ustar_header(&ustar_name, mode, header_size, typeflag);
        self.write_all_bytes(&header_block)
    }

    fn write_file(
        &mut self,
        rel_posix: &str,
        full_path: &Path,
        mode: u32,
        size: u64,
    ) -> Result<(), String> {
        self.write_header(rel_posix, mode, size, b'0')?;

        let mut src = File::open(full_path).map_err(|err| {
            format!(
                "failed to open `{}` for archiving: {err}",
                full_path.display()
            )
        })?;
        let mut buf = [0u8; COPY_BUF_SIZE];
        let mut remaining = size;
        while remaining > 0 {
            let to_read = (remaining as usize).min(buf.len());
            let n = src
                .read(&mut buf[..to_read])
                .map_err(|err| format!("failed to read `{}`: {err}", full_path.display()))?;
            if n == 0 {
                return Err(format!(
                    "file `{}` shrank while archiving",
                    full_path.display()
                ));
            }
            self.write_all_bytes(&buf[..n])?;
            remaining -= n as u64;
        }
        self.write_padding(size)
    }

    fn write_padding(&mut self, size: u64) -> Result<(), String> {
        let rem = (size % BLOCK_SIZE as u64) as usize;
        if rem != 0 {
            let pad = [0u8; BLOCK_SIZE];
            self.write_all_bytes(&pad[..BLOCK_SIZE - rem])?;
        }
        Ok(())
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.inner
            .write_all(bytes)
            .map_err(|err| format!("failed to write tar data: {err}"))
    }

    fn finish(mut self) -> io::Result<()> {
        // End-of-archive marker: two consecutive 512-byte zero blocks.
        let zeros = [0u8; BLOCK_SIZE * 2];
        self.inner.write_all(&zeros)?;
        let mut buf_writer = self.inner.finish()?;
        buf_writer.flush()
    }
}

fn format_pax_record(key: &str, value: &str) -> Vec<u8> {
    let body_len = 1 + key.len() + 1 + value.len() + 1;
    let mut total = body_len + 1;
    loop {
        let digits = total.to_string().len();
        if digits + body_len == total {
            break;
        }
        total = digits + body_len;
    }
    format!("{total} {key}={value}\n").into_bytes()
}

fn pax_header_name(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or("entry");
    let safe = truncate_ascii_name(base, 80);
    format!("PaxHeaders/{safe}")
}

fn truncate_ascii_name(input: &str, max_len: usize) -> String {
    let mut out = String::with_capacity(max_len);
    for ch in input.chars() {
        let mapped = if ch.is_ascii() && !ch.is_ascii_control() {
            ch
        } else {
            '_'
        };
        if out.len() >= max_len {
            break;
        }
        out.push(mapped);
    }
    if out.is_empty() {
        "entry".to_owned()
    } else {
        out
    }
}

fn build_ustar_header(name: &str, mode: u32, size: u64, typeflag: u8) -> [u8; BLOCK_SIZE] {
    let mut block = [0u8; BLOCK_SIZE];
    let name_bytes = name.as_bytes();
    let copy_len = name_bytes.len().min(100);
    block[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

    write_octal(&mut block[100..108], u64::from(mode));
    write_octal(&mut block[108..116], 0);
    write_octal(&mut block[116..124], 0);
    write_octal(&mut block[124..136], size);
    write_octal(&mut block[136..148], 0);

    // Fill checksum field with spaces prior to computing checksum.
    block[148..156].fill(b' ');
    block[156] = typeflag;

    // USTAR magic and version.
    block[257..263].copy_from_slice(b"ustar\0");
    block[263..265].copy_from_slice(b"00");
    write_octal(&mut block[329..337], 0);
    write_octal(&mut block[337..345], 0);

    let checksum: u32 = block.iter().map(|&b| u32::from(b)).sum();
    let chk_str = format!("{checksum:06o}\0 ");
    block[148..156].copy_from_slice(chk_str.as_bytes());
    block
}

fn write_octal(slot: &mut [u8], value: u64) {
    let width = slot.len() - 1;
    let formatted = format!("{value:0width$o}");
    let bytes = formatted.as_bytes();
    let start = bytes.len().saturating_sub(width);
    slot[..width].copy_from_slice(&bytes[start..]);
    slot[width] = 0;
}

struct ParsedHeader {
    name: String,
    mode: u32,
    size: u64,
    typeflag: u8,
    linkname: String,
}

fn read_header_block<R: Read>(reader: &mut R, buf: &mut [u8; BLOCK_SIZE]) -> Result<bool, String> {
    let mut filled = 0;
    while filled < BLOCK_SIZE {
        let n = reader
            .read(&mut buf[filled..])
            .map_err(|err| format!("failed to read tar header: {err}"))?;
        if n == 0 {
            if filled == 0 {
                return Ok(false);
            }
            return Err("truncated tar header block".to_owned());
        }
        filled += n;
    }
    Ok(true)
}

fn verify_checksum(block: &[u8; BLOCK_SIZE]) -> Result<(), String> {
    let stored = parse_octal(&block[148..156])?;
    let mut unsigned_sum: u64 = 0;
    let mut signed_sum: i64 = 0;
    for (idx, &byte) in block.iter().enumerate() {
        let effective = if (148..156).contains(&idx) {
            b' '
        } else {
            byte
        };
        unsigned_sum += u64::from(effective);
        signed_sum += i64::from(effective as i8);
    }
    if stored != unsigned_sum && (signed_sum < 0 || stored != signed_sum as u64) {
        return Err(format!(
            "tar header checksum mismatch (stored {stored}, computed {unsigned_sum})"
        ));
    }
    Ok(())
}

fn parse_header(block: &[u8; BLOCK_SIZE]) -> Result<ParsedHeader, String> {
    let name_part = trim_nul_utf8(&block[0..100])?;
    let prefix_part = if &block[257..262] == b"ustar" {
        trim_nul_utf8(&block[345..500])?
    } else {
        String::new()
    };
    let name = if prefix_part.is_empty() {
        name_part
    } else {
        format!("{prefix_part}/{name_part}")
    };
    let mode = parse_octal(&block[100..108])? as u32;
    let size = parse_tar_size(&block[124..136])?;
    let typeflag = match block[156] {
        0 => b'0',
        other => other,
    };
    let linkname = trim_nul_utf8(&block[157..257])?;

    Ok(ParsedHeader {
        name,
        mode,
        size,
        typeflag,
        linkname,
    })
}

fn trim_nul_utf8(bytes: &[u8]) -> Result<String, String> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let text = std::str::from_utf8(&bytes[..end])
        .map_err(|err| format!("tar header contains invalid UTF-8: {err}"))?;
    Ok(text.to_owned())
}

fn parse_octal(field: &[u8]) -> Result<u64, String> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    let text = std::str::from_utf8(&field[..end])
        .map_err(|err| format!("non-UTF-8 octal field in tar header: {err}"))?
        .trim();
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8)
        .map_err(|err| format!("invalid octal value `{text}` in tar header: {err}"))
}

fn parse_tar_size(field: &[u8]) -> Result<u64, String> {
    // Support GNU base-256 extension (high bit set on first byte) as well as standard octal.
    if field[0] & 0x80 != 0 {
        let mut value: u64 = u64::from(field[0] & 0x7f);
        for &byte in &field[1..] {
            value = value
                .checked_shl(8)
                .ok_or_else(|| "tar entry size overflows u64".to_owned())?
                | u64::from(byte);
        }
        return Ok(value);
    }
    parse_octal(field)
}

fn read_exact_entry_bytes<R: Read>(reader: &mut R, size: u64) -> Result<Vec<u8>, String> {
    let len =
        usize::try_from(size).map_err(|_| format!("tar metadata entry too large: {size} bytes"))?;
    let mut data = vec![0u8; len];
    reader
        .read_exact(&mut data)
        .map_err(|err| format!("truncated tar metadata entry: {err}"))?;
    skip_padding(reader, size)?;
    Ok(data)
}

fn skip_padding<R: Read>(reader: &mut R, size: u64) -> Result<(), String> {
    let rem = (size % BLOCK_SIZE as u64) as usize;
    if rem != 0 {
        let mut discard = [0u8; BLOCK_SIZE];
        reader
            .read_exact(&mut discard[..BLOCK_SIZE - rem])
            .map_err(|err| format!("truncated tar block padding: {err}"))?;
    }
    Ok(())
}

fn parse_pax_records(data: &[u8], out: &mut BTreeMap<String, String>) -> Result<(), String> {
    let mut cursor = 0;
    while cursor < data.len() {
        let Some(space_rel) = data[cursor..].iter().position(|&b| b == b' ') else {
            return Err("malformed PAX record: missing length delimiter".to_owned());
        };
        let len_str = std::str::from_utf8(&data[cursor..cursor + space_rel])
            .map_err(|err| format!("invalid PAX record length: {err}"))?;
        let record_len: usize = len_str
            .parse()
            .map_err(|err| format!("invalid PAX record length `{len_str}`: {err}"))?;
        if record_len == 0 || cursor + record_len > data.len() {
            return Err(format!("invalid PAX record length {record_len}"));
        }

        let body_start = cursor + space_rel + 1;
        let body_end = (cursor + record_len).saturating_sub(1); // strip trailing '\n'
        if body_start <= body_end {
            let body = std::str::from_utf8(&data[body_start..body_end])
                .map_err(|err| format!("non-UTF-8 PAX record: {err}"))?;
            if let Some((key, value)) = body.split_once('=') {
                out.insert(key.to_owned(), value.to_owned());
            }
        }
        cursor += record_len;
    }
    Ok(())
}

fn extract_single_entry<R: Read>(
    reader: &mut R,
    dest_dir: &Path,
    entry_path_raw: &str,
    link_target_raw: &str,
    typeflag: u8,
    mode: u32,
    size: u64,
) -> Result<(), String> {
    let rel_path = sanitize_archive_path(entry_path_raw)?;

    // Root directory entry (e.g. `./` or `.`): nothing to create.
    if rel_path.as_os_str().is_empty() {
        if size > 0 {
            drain_entry(reader, size)?;
        }
        return Ok(());
    }

    let out_path = dest_dir.join(&rel_path);
    match typeflag {
        b'5' => {
            fs::create_dir_all(&out_path)
                .map_err(|err| format!("failed to create dir `{}`: {err}", out_path.display()))?;
            if size > 0 {
                drain_entry(reader, size)?;
            }
        }
        b'0' => {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create dir `{}`: {err}", parent.display()))?;
            }
            let mut out_file = File::create(&out_path)
                .map_err(|err| format!("failed to create `{}`: {err}", out_path.display()))?;
            copy_entry_bytes(reader, &mut out_file, size, &out_path)?;
            drop(out_file);
            apply_unix_mode(&out_path, mode)?;
        }
        b'2' => {
            validate_symlink_target(&rel_path, link_target_raw)?;
            if size > 0 {
                drain_entry(reader, size)?;
            }
            create_safe_symlink(&out_path, link_target_raw)?;
        }
        b'1' => {
            let link_rel = sanitize_archive_path(link_target_raw)?;
            if link_rel.as_os_str().is_empty() {
                return Err(format!(
                    "unsafe empty hardlink target for `{entry_path_raw}`"
                ));
            }
            if size > 0 {
                drain_entry(reader, size)?;
            }
            let target_full = dest_dir.join(&link_rel);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create dir `{}`: {err}", parent.display()))?;
            }
            fs::hard_link(&target_full, &out_path).map_err(|err| {
                format!(
                    "failed to create hardlink `{}` -> `{}`: {err}",
                    out_path.display(),
                    target_full.display()
                )
            })?;
        }
        other => {
            return Err(format!(
                "unsupported tar entry type '{}' for `{entry_path_raw}`",
                other as char
            ));
        }
    }
    Ok(())
}

/// Strictly validate an archive member path, rejecting absolute paths, drive prefixes,
/// backslashes, NUL bytes, and any `..` traversal components.
fn sanitize_archive_path(raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() {
        return Err("archive contains an entry with an empty path".to_owned());
    }
    if raw.contains('\0') {
        return Err("archive path contains a NUL byte".to_owned());
    }
    if raw.starts_with('/') || raw.starts_with('\\') {
        return Err(format!(
            "archive path `{raw}` rejected: absolute paths are forbidden"
        ));
    }
    if raw.contains('\\') {
        return Err(format!(
            "archive path `{raw}` rejected: backslashes are forbidden"
        ));
    }

    let mut clean = PathBuf::new();
    for segment in raw.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return Err(format!(
                "archive path `{raw}` rejected: path traversal (`..`) is forbidden"
            ));
        }
        if segment.contains(':') {
            return Err(format!(
                "archive path `{raw}` rejected: drive/prefix specifier is forbidden"
            ));
        }
        clean.push(segment);
    }

    for component in clean.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(format!(
                "archive path `{raw}` rejected: unsafe path component"
            ));
        }
    }
    Ok(clean)
}

/// Verify that a symlink at `entry_rel` pointing to `target` stays strictly inside `dest_dir`.
fn validate_symlink_target(entry_rel: &Path, target: &str) -> Result<(), String> {
    if target.is_empty()
        || target.contains('\0')
        || target.starts_with('/')
        || target.starts_with('\\')
        || target.contains('\\')
    {
        return Err(format!(
            "archive symlink `{}` -> `{target}` rejected: unsafe target",
            entry_rel.display()
        ));
    }

    let mut depth: Vec<String> = entry_rel
        .parent()
        .map(|p| {
            p.components()
                .filter_map(|c| match c {
                    Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    for segment in target.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment.contains(':') {
            return Err(format!(
                "archive symlink `{}` -> `{target}` rejected: drive prefix",
                entry_rel.display()
            ));
        }
        if segment == ".." {
            if depth.pop().is_none() {
                return Err(format!(
                    "archive symlink `{}` -> `{target}` rejected: escapes destination directory",
                    entry_rel.display()
                ));
            }
        } else {
            depth.push(segment.to_owned());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn create_safe_symlink(out_path: &Path, target: &str) -> Result<(), String> {
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create dir `{}`: {err}", parent.display()))?;
    }
    if out_path.symlink_metadata().is_ok() {
        let _ = fs::remove_file(out_path);
    }
    std::os::unix::fs::symlink(target, out_path).map_err(|err| {
        format!(
            "failed to create symlink `{}` -> `{target}`: {err}",
            out_path.display()
        )
    })
}

#[cfg(not(unix))]
fn create_safe_symlink(out_path: &Path, target: &str) -> Result<(), String> {
    Err(format!(
        "archive symlink `{}` -> `{target}` is not supported on this platform",
        out_path.display()
    ))
}

fn copy_entry_bytes<R: Read>(
    reader: &mut R,
    out_file: &mut File,
    size: u64,
    out_path: &Path,
) -> Result<(), String> {
    let mut buf = [0u8; COPY_BUF_SIZE];
    let mut remaining = size;
    while remaining > 0 {
        let chunk = (remaining as usize).min(buf.len());
        let n = reader
            .read(&mut buf[..chunk])
            .map_err(|err| format!("failed to read tar entry `{}`: {err}", out_path.display()))?;
        if n == 0 {
            return Err(format!(
                "truncated tar entry `{}` ({remaining} bytes remaining)",
                out_path.display()
            ));
        }
        out_file
            .write_all(&buf[..n])
            .map_err(|err| format!("failed to write `{}`: {err}", out_path.display()))?;
        remaining -= n as u64;
    }
    skip_padding(reader, size)
}

fn drain_entry<R: Read>(reader: &mut R, size: u64) -> Result<(), String> {
    let mut buf = [0u8; COPY_BUF_SIZE];
    let mut remaining = size;
    while remaining > 0 {
        let chunk = (remaining as usize).min(buf.len());
        let n = reader
            .read(&mut buf[..chunk])
            .map_err(|err| format!("failed to skip tar entry data: {err}"))?;
        if n == 0 {
            return Err("truncated tar entry while skipping payload".to_owned());
        }
        remaining -= n as u64;
    }
    skip_padding(reader, size)
}

#[cfg(unix)]
fn apply_unix_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let effective = if mode & 0o777 == 0 {
        0o644
    } else {
        (mode & 0o777) | 0o600
    };
    fs::set_permissions(path, fs::Permissions::from_mode(effective))
        .map_err(|err| format!("failed to set permissions on `{}`: {err}", path.display()))
}

#[cfg(not(unix))]
fn apply_unix_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
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
            "cargo-mires-archive-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trips_long_paths_nested_dirs_and_executable_modes() {
        let root = unique_temp_dir("roundtrip");
        let src = root.join("src");
        let dst = root.join("dst");
        let archive = root.join("bundle.tar.gz");

        let long_rel = format!(
            "sysroots/{}/lib/rustlib/mires32-unwind/lib/libcompiler_builtins-0123456789abcdef.rlib",
            "a".repeat(64)
        );
        assert!(long_rel.len() > 120);

        let long_file = src.join(&long_rel);
        fs::create_dir_all(long_file.parent().unwrap()).unwrap();
        fs::write(&long_file, b"rlib payload bytes 0123456789").unwrap();

        let bin_file = src.join("bin/jsc-link");
        fs::create_dir_all(bin_file.parent().unwrap()).unwrap();
        fs::write(&bin_file, b"#!/bin/sh\necho ok\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&bin_file, fs::Permissions::from_mode(0o755)).unwrap();
        }

        fs::create_dir_all(src.join("empty/sub")).unwrap();

        create_tar_gz(&src, &archive).unwrap();
        extract_tar_gz(&archive, &dst).unwrap();

        assert_eq!(
            fs::read(dst.join(&long_rel)).unwrap(),
            b"rlib payload bytes 0123456789"
        );
        assert_eq!(
            fs::read(dst.join("bin/jsc-link")).unwrap(),
            b"#!/bin/sh\necho ok\n"
        );
        assert!(dst.join("empty/sub").is_dir());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dst.join("bin/jsc-link"))
                .unwrap()
                .permissions()
                .mode();
            assert_ne!(mode & 0o111, 0, "expected executable bit to be preserved");
        }

        let _ = fs::remove_dir_all(&root);
    }

    fn write_crafted_tar_gz(archive_path: &Path, header: &[u8; BLOCK_SIZE], body: &[u8]) {
        let file = File::create(archive_path).unwrap();
        let mut gz = GzEncoder::new(file, Compression::default());
        gz.write_all(header).unwrap();
        gz.write_all(body).unwrap();
        let rem = body.len() % BLOCK_SIZE;
        if rem != 0 {
            gz.write_all(&vec![0u8; BLOCK_SIZE - rem]).unwrap();
        }
        gz.write_all(&[0u8; BLOCK_SIZE * 2]).unwrap();
        gz.finish().unwrap();
    }

    #[test]
    fn rejects_path_traversal_absolute_paths_and_escaping_symlinks() {
        let root = unique_temp_dir("security");

        // 1. Parent traversal (`../escape.txt`)
        let traversal_tgz = root.join("traversal.tar.gz");
        let h1 = build_ustar_header("../escape.txt", 0o644, 4, b'0');
        write_crafted_tar_gz(&traversal_tgz, &h1, b"evil");
        assert!(extract_tar_gz(&traversal_tgz, &root.join("out1")).is_err());

        // 2. Absolute path (`/tmp/evil.txt`)
        let abs_tgz = root.join("abs.tar.gz");
        let h2 = build_ustar_header("/tmp/evil.txt", 0o644, 4, b'0');
        write_crafted_tar_gz(&abs_tgz, &h2, b"evil");
        assert!(extract_tar_gz(&abs_tgz, &root.join("out2")).is_err());

        // 3. Symlink escaping destination (`bin/link -> ../../etc/passwd`)
        let sym_tgz = root.join("sym.tar.gz");
        let mut h3 = build_ustar_header("bin/link", 0o777, 0, b'2');
        let target = b"../../etc/passwd";
        h3[157..157 + target.len()].copy_from_slice(target);
        // Recompute checksum after setting linkname.
        h3[148..156].fill(b' ');
        let chk: u32 = h3.iter().map(|&b| u32::from(b)).sum();
        h3[148..156].copy_from_slice(format!("{chk:06o}\0 ").as_bytes());
        write_crafted_tar_gz(&sym_tgz, &h3, b"");
        assert!(extract_tar_gz(&sym_tgz, &root.join("out3")).is_err());

        let _ = fs::remove_dir_all(&root);
    }
}
