//! Cross-platform POSIX-style path manipulation (`Path`, `PathBuf`, `Component`, `Components`).

use crate::borrow::{Borrow, Cow, ToOwned};
use crate::boxed::Box;
use crate::cmp::Ordering;
use crate::convert::Infallible;
use crate::error::Error;
use crate::ffi::{OsStr, OsString};
use crate::fmt;
use crate::hash::{Hash, Hasher};
use crate::iter::FusedIterator;
use crate::ops::Deref;
use crate::str::FromStr;
use crate::string::String;

/// The primary separator of path components (`'/'`).
pub const MAIN_SEPARATOR: char = '/';

/// The primary separator of path components as a string (`"/"`).
pub const MAIN_SEPARATOR_STR: &str = "/";

/// Determines whether the character is one of the permitted path separators.
#[must_use]
#[inline]
pub fn is_separator(c: char) -> bool {
    c == '/'
}

/// An error returned from [`Path::strip_prefix`] if the prefix was not found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripPrefixError(());

impl fmt::Display for StripPrefixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("prefix not found")
    }
}

impl Error for StripPrefixError {}

/// Windows path prefixes, e.g., `C:` or `\\server\share`.
///
/// Provided for cross-platform API compatibility with upstream `std::path`;
/// POSIX/JS paths do not produce prefix components.
#[derive(Copy, Clone, Debug, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub enum Prefix<'a> {
    /// Verbatim prefix, e.g., `\\?\cat_pics`.
    Verbatim(&'a OsStr),
    /// Verbatim prefix using Windows' UNC, e.g., `\\?\UNC\server\share`.
    VerbatimUNC(&'a OsStr, &'a OsStr),
    /// Verbatim disk prefix, e.g., `\\?\C:`.
    VerbatimDisk(u8),
    /// Device namespace prefix, e.g., `\\.\COM42`.
    DeviceNS(&'a OsStr),
    /// UNC prefix, e.g., `\\server\share`.
    UNC(&'a OsStr, &'a OsStr),
    /// Prefix `C:` for the given disk drive.
    Disk(u8),
}

impl<'a> Prefix<'a> {
    /// Determines if the prefix is verbatim, i.e., begins with `\\?\`.
    #[inline]
    #[must_use]
    pub fn is_verbatim(&self) -> bool {
        matches!(
            *self,
            Prefix::Verbatim(_) | Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(..)
        )
    }
}

/// A structure wrapping a Windows path prefix as well as its unparsed string representation.
///
/// Does not occur on Unix or JavaScript targets.
#[derive(Copy, Clone, Eq, Debug)]
pub struct PrefixComponent<'a> {
    raw: &'a OsStr,
    parsed: Prefix<'a>,
}

impl<'a> PrefixComponent<'a> {
    /// Returns the parsed prefix data.
    #[must_use]
    #[inline]
    pub fn kind(&self) -> Prefix<'a> {
        self.parsed
    }

    /// Returns the raw [`OsStr`] slice for this prefix.
    #[must_use]
    #[inline]
    pub fn as_os_str(&self) -> &'a OsStr {
        self.raw
    }
}

impl<'a> PartialEq for PrefixComponent<'a> {
    #[inline]
    fn eq(&self, other: &PrefixComponent<'a>) -> bool {
        self.parsed == other.parsed
    }
}

impl<'a> PartialOrd for PrefixComponent<'a> {
    #[inline]
    fn partial_cmp(&self, other: &PrefixComponent<'a>) -> Option<Ordering> {
        PartialOrd::partial_cmp(&self.parsed, &other.parsed)
    }
}

impl Ord for PrefixComponent<'_> {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        Ord::cmp(&self.parsed, &other.parsed)
    }
}

impl Hash for PrefixComponent<'_> {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.parsed.hash(h);
    }
}

/// A single component of a path.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Component<'a> {
    /// A Windows path prefix, e.g., `C:` or `\\server\share`. Does not occur on JS/POSIX.
    Prefix(PrefixComponent<'a>),
    /// The root directory component (`/`).
    RootDir,
    /// A reference to the current directory (`.`).
    CurDir,
    /// A reference to the parent directory (`..`).
    ParentDir,
    /// A normal path component (file or directory name).
    Normal(&'a OsStr),
}

impl<'a> Component<'a> {
    /// Extracts the underlying [`OsStr`] slice.
    #[must_use]
    pub fn as_os_str(self) -> &'a OsStr {
        match self {
            Component::Prefix(p) => p.as_os_str(),
            Component::RootDir => OsStr::new("/"),
            Component::CurDir => OsStr::new("."),
            Component::ParentDir => OsStr::new(".."),
            Component::Normal(s) => s,
        }
    }
}

impl AsRef<OsStr> for Component<'_> {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        self.as_os_str()
    }
}

impl AsRef<Path> for Component<'_> {
    #[inline]
    fn as_ref(&self) -> &Path {
        Path::new(self.as_os_str())
    }
}

/// Helper struct for safely printing [`Path`] values with [`fmt::Display`].
pub struct Display<'a> {
    path: &'a Path,
}

impl fmt::Debug for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.path.as_str(), f)
    }
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.path.as_str(), f)
    }
}

/// An iterator over the [`Component`]s of a [`Path`].
#[derive(Clone)]
pub struct Components<'a> {
    path: &'a str,
    has_root: bool,
    emitted_root: bool,
    emitted_curdir_prefix: bool,
    front_started: bool,
}

impl<'a> Components<'a> {
    fn new(path: &'a str) -> Self {
        let has_root = path.starts_with('/');
        let rest = if has_root {
            path.trim_start_matches('/')
        } else {
            path
        };
        let rest = trim_trailing_slashes(rest);
        let leading_curdir = !has_root && (rest == "." || rest.starts_with("./"));
        let rest = if leading_curdir {
            rest.strip_prefix('.').unwrap_or(rest).trim_start_matches('/')
        } else {
            rest
        };
        Components {
            path: rest,
            has_root,
            emitted_root: !has_root,
            emitted_curdir_prefix: !leading_curdir,
            front_started: false,
        }
    }

    /// Extracts a slice corresponding to the portion of the path remaining for iteration.
    #[must_use]
    pub fn as_path(&self) -> &'a Path {
        if !self.emitted_root {
            if self.path.is_empty() {
                return Path::from_str("/");
            }
            return Path::from_str(self.path);
        }
        Path::from_str(self.path)
    }

    fn parse_component(segment: &'a str) -> Option<Component<'a>> {
        match segment {
            "" | "." => None,
            ".." => Some(Component::ParentDir),
            s => Some(Component::Normal(OsStr::new(s))),
        }
    }
}

fn trim_trailing_slashes(s: &str) -> &str {
    s.trim_end_matches('/')
}

#[inline]
fn find_byte(s: &str, needle: u8) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[inline]
fn rfind_byte(s: &str, needle: u8) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == needle {
            return Some(i);
        }
    }
    None
}

impl<'a> Iterator for Components<'a> {
    type Item = Component<'a>;

    fn next(&mut self) -> Option<Component<'a>> {
        if !self.emitted_root {
            self.emitted_root = true;
            self.front_started = true;
            return Some(Component::RootDir);
        }
        if !self.emitted_curdir_prefix {
            self.emitted_curdir_prefix = true;
            self.front_started = true;
            return Some(Component::CurDir);
        }
        self.front_started = true;
        loop {
            let trimmed = self.path.trim_start_matches('/');
            if trimmed.is_empty() {
                self.path = "";
                return None;
            }
            let (seg, rest) = match find_byte(trimmed, b'/') {
                Some(idx) => (&trimmed[..idx], &trimmed[idx + 1..]),
                None => (trimmed, ""),
            };
            self.path = rest;
            if let Some(comp) = Self::parse_component(seg) {
                return Some(comp);
            }
        }
    }
}

impl<'a> DoubleEndedIterator for Components<'a> {
    fn next_back(&mut self) -> Option<Component<'a>> {
        loop {
            let trimmed = self.path.trim_end_matches('/');
            if trimmed.is_empty() {
                self.path = "";
                if !self.emitted_curdir_prefix {
                    self.emitted_curdir_prefix = true;
                    return Some(Component::CurDir);
                }
                if !self.emitted_root {
                    self.emitted_root = true;
                    return Some(Component::RootDir);
                }
                return None;
            }
            let (rest, seg) = match rfind_byte(trimmed, b'/') {
                Some(idx) => (&trimmed[..idx], &trimmed[idx + 1..]),
                None => ("", trimmed),
            };
            self.path = rest;
            if let Some(comp) = Self::parse_component(seg) {
                return Some(comp);
            }
        }
    }
}

impl FusedIterator for Components<'_> {}

impl fmt::Debug for Components<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

impl PartialEq for Components<'_> {
    #[inline]
    fn eq(&self, other: &Components<'_>) -> bool {
        self.clone().eq(other.clone())
    }
}

impl Eq for Components<'_> {}

impl PartialOrd for Components<'_> {
    #[inline]
    fn partial_cmp(&self, other: &Components<'_>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Components<'_> {
    #[inline]
    fn cmp(&self, other: &Components<'_>) -> Ordering {
        self.clone().cmp(other.clone())
    }
}

/// An iterator over the [`OsStr`] components of a [`Path`].
#[derive(Clone, Debug)]
pub struct Iter<'a> {
    inner: Components<'a>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a OsStr;

    #[inline]
    fn next(&mut self) -> Option<&'a OsStr> {
        self.inner.next().map(Component::as_os_str)
    }
}

impl<'a> DoubleEndedIterator for Iter<'a> {
    #[inline]
    fn next_back(&mut self) -> Option<&'a OsStr> {
        self.inner.next_back().map(Component::as_os_str)
    }
}

impl FusedIterator for Iter<'_> {}

/// An iterator over [`Path`] and its ancestors.
#[derive(Copy, Clone, Debug)]
pub struct Ancestors<'a> {
    next: Option<&'a Path>,
}

impl<'a> Iterator for Ancestors<'a> {
    type Item = &'a Path;

    #[inline]
    fn next(&mut self) -> Option<&'a Path> {
        let next = self.next;
        self.next = next.and_then(Path::parent);
        next
    }
}

impl FusedIterator for Ancestors<'_> {}

/// A slice of a path (akin to [`str`]).
#[repr(transparent)]
pub struct Path {
    inner: OsStr,
}

/// An owned, mutable path (akin to [`String`]).
#[derive(Clone, Default)]
pub struct PathBuf {
    inner: OsString,
}

impl Path {
    /// Directly wraps a string slice as a `Path` slice.
    #[must_use]
    #[inline]
    pub fn new<S: AsRef<OsStr> + ?Sized>(s: &S) -> &Path {
        // SAFETY: `Path` is `#[repr(transparent)]` over `OsStr`.
        unsafe { &*(s.as_ref() as *const OsStr as *const Path) }
    }

    #[inline]
    fn from_str(s: &str) -> &Path {
        Path::new(OsStr::new(s))
    }

    #[inline]
    fn as_str(&self) -> &str {
        self.inner.to_str().unwrap_or("")
    }

    /// Yields the underlying [`OsStr`] slice.
    #[must_use]
    #[inline]
    pub fn as_os_str(&self) -> &OsStr {
        &self.inner
    }

    /// Yields a `&str` slice if the `Path` is valid Unicode.
    #[must_use]
    #[inline]
    pub fn to_str(&self) -> Option<&str> {
        self.inner.to_str()
    }

    /// Converts a `Path` to a [`Cow<str>`].
    #[must_use]
    #[inline]
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        self.inner.to_string_lossy()
    }

    /// Converts a `Path` to an owned [`PathBuf`].
    #[must_use]
    #[inline]
    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf {
            inner: self.inner.to_os_string(),
        }
    }

    /// Returns `true` if the `Path` is absolute (starts with `'/'`).
    #[must_use]
    #[inline]
    pub fn is_absolute(&self) -> bool {
        self.has_root()
    }

    /// Returns `true` if the `Path` is relative.
    #[must_use]
    #[inline]
    pub fn is_relative(&self) -> bool {
        !self.is_absolute()
    }

    /// Returns `true` if the `Path` has a root (`'/'`).
    #[must_use]
    #[inline]
    pub fn has_root(&self) -> bool {
        self.as_str().starts_with('/')
    }

    /// Returns the `Path` without its final component, if there is one.
    #[must_use]
    pub fn parent(&self) -> Option<&Path> {
        let s = self.as_str();
        if s.is_empty() {
            return None;
        }
        let trimmed = s.trim_end_matches('/');
        if trimmed.is_empty() {
            // Path was "/" or "///" — root has no parent.
            return None;
        }
        match rfind_byte(trimmed, b'/') {
            None => Some(Path::from_str("")),
            Some(0) => Some(Path::from_str("/")),
            Some(idx) => {
                let prefix = trimmed[..idx].trim_end_matches('/');
                if prefix.is_empty() {
                    Some(Path::from_str("/"))
                } else {
                    Some(Path::from_str(prefix))
                }
            }
        }
    }

    /// Produces an iterator over `Path` and its ancestors.
    #[must_use]
    #[inline]
    pub fn ancestors(&self) -> Ancestors<'_> {
        Ancestors { next: Some(self) }
    }

    /// Returns the final component of the `Path`, if it is a normal file name.
    #[must_use]
    pub fn file_name(&self) -> Option<&OsStr> {
        match self.components().next_back() {
            Some(Component::Normal(name)) => Some(name),
            _ => None,
        }
    }

    /// Returns a path that, when joined onto `base`, yields `self`.
    pub fn strip_prefix<P>(&self, base: P) -> Result<&Path, StripPrefixError>
    where
        P: AsRef<Path>,
    {
        let mut self_comps = self.components();
        for base_comp in base.as_ref().components() {
            match self_comps.next() {
                Some(c) if c == base_comp => {}
                _ => return Err(StripPrefixError(())),
            }
        }
        Ok(self_comps.as_path())
    }

    /// Determines whether `base` is a prefix of `self`.
    #[must_use]
    pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool {
        self.strip_prefix(base).is_ok()
    }

    /// Determines whether `child` is a suffix of `self`.
    #[must_use]
    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        let mut self_comps = self.components();
        for child_comp in child.as_ref().components().rev() {
            match self_comps.next_back() {
                Some(c) if c == child_comp => {}
                _ => return false,
            }
        }
        true
    }

    /// Extracts the stem (non-extension) portion of [`Path::file_name`].
    #[must_use]
    pub fn file_stem(&self) -> Option<&OsStr> {
        let name = self.file_name()?.to_str()?;
        let (stem, _) = split_file_at_dot(name);
        Some(OsStr::new(stem))
    }

    /// Extracts the extension of [`Path::file_name`], if possible.
    #[must_use]
    pub fn extension(&self) -> Option<&OsStr> {
        let name = self.file_name()?.to_str()?;
        let (_, ext) = split_file_at_dot(name);
        ext.map(OsStr::new)
    }

    /// Creates an owned [`PathBuf`] with `path` adjoined to `self`.
    #[must_use]
    pub fn join<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        let mut buf = self.to_path_buf();
        buf.push(path);
        buf
    }

    /// Creates an owned [`PathBuf`] like `self` but with the given file name.
    #[must_use]
    pub fn with_file_name<S: AsRef<OsStr>>(&self, file_name: S) -> PathBuf {
        let mut buf = self.to_path_buf();
        buf.set_file_name(file_name);
        buf
    }

    /// Creates an owned [`PathBuf`] like `self` but with the given extension.
    #[must_use]
    pub fn with_extension<S: AsRef<OsStr>>(&self, extension: S) -> PathBuf {
        let mut buf = self.to_path_buf();
        buf.set_extension(extension);
        buf
    }

    /// Produces an iterator over the [`Component`]s of the path.
    #[must_use]
    #[inline]
    pub fn components(&self) -> Components<'_> {
        Components::new(self.as_str())
    }

    /// Produces an iterator over the path's components viewed as [`OsStr`] slices.
    #[must_use]
    #[inline]
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            inner: self.components(),
        }
    }

    /// Returns an object that implements [`fmt::Display`] for safely printing paths.
    #[must_use]
    #[inline]
    pub fn display(&self) -> Display<'_> {
        Display { path: self }
    }

    /// Returns `true` if the path points at an existing entity (always `false` on JS).
    #[must_use]
    #[inline]
    pub fn exists(&self) -> bool {
        false
    }

    /// Returns `true` if the path points at an existing file (always `false` on JS).
    #[must_use]
    #[inline]
    pub fn is_file(&self) -> bool {
        false
    }

    /// Returns `true` if the path points at an existing directory (always `false` on JS).
    #[must_use]
    #[inline]
    pub fn is_dir(&self) -> bool {
        false
    }

    /// Returns `true` if the path points at a symbolic link (always `false` on JS).
    #[must_use]
    #[inline]
    pub fn is_symlink(&self) -> bool {
        false
    }

    /// Queries the filesystem to get information about a file (`Unsupported` on JS).
    #[inline]
    pub fn metadata(&self) -> crate::io::Result<crate::fs::Metadata> {
        crate::fs::metadata(self)
    }

    /// Returns the canonical form of the path (`Unsupported` on JS).
    #[inline]
    pub fn canonicalize(&self) -> crate::io::Result<PathBuf> {
        crate::fs::canonicalize(self)
    }

    /// Returns an iterator over directory entries (`Unsupported` on JS).
    #[inline]
    pub fn read_dir(&self) -> crate::io::Result<crate::fs::ReadDir> {
        crate::fs::read_dir(self)
    }
}

fn split_file_at_dot(file: &str) -> (&str, Option<&str>) {
    if file == ".." {
        return (file, None);
    }
    match rfind_byte(file, b'.') {
        None | Some(0) => (file, None),
        Some(idx) => (&file[..idx], Some(&file[idx + 1..])),
    }
}

impl PathBuf {
    /// Allocates an empty `PathBuf`.
    #[must_use]
    #[inline]
    pub const fn new() -> PathBuf {
        PathBuf {
            inner: OsString::new(),
        }
    }

    /// Creates a new `PathBuf` with a given capacity.
    #[must_use]
    #[inline]
    pub fn with_capacity(capacity: usize) -> PathBuf {
        PathBuf {
            inner: OsString::with_capacity(capacity),
        }
    }

    /// Coerces to a [`Path`] slice.
    #[must_use]
    #[inline]
    pub fn as_path(&self) -> &Path {
        Path::new(&self.inner)
    }

    /// Extends `self` with `path`. If `path` is absolute, it replaces the current path.
    pub fn push<P: AsRef<Path>>(&mut self, path: P) {
        let other = path.as_ref();
        if other.is_absolute() {
            self.inner.clear();
            self.inner.push(other.as_os_str());
            return;
        }
        let need_sep = match self.as_str().as_bytes().last() {
            None | Some(b'/') => false,
            Some(_) => !other.as_str().is_empty(),
        };
        if need_sep {
            self.inner.push("/");
        }
        self.inner.push(other.as_os_str());
    }

    /// Truncates `self` to [`Path::parent`]. Returns `false` if there is no parent.
    pub fn pop(&mut self) -> bool {
        match self.parent().map(|p| p.as_str().len()) {
            Some(new_len) => {
                let mut s = crate::mem::take(&mut self.inner)
                    .into_string()
                    .unwrap_or_default();
                s.truncate(new_len);
                self.inner = OsString::from(s);
                true
            }
            None => false,
        }
    }

    /// Updates [`Path::file_name`] to `file_name`.
    pub fn set_file_name<S: AsRef<OsStr>>(&mut self, file_name: S) {
        if self.file_name().is_some() {
            let _ = self.pop();
        }
        self.push(file_name.as_ref());
    }

    /// Updates [`Path::extension`] to `extension`.
    pub fn set_extension<S: AsRef<OsStr>>(&mut self, extension: S) -> bool {
        let Some(file_stem) = self.file_stem().and_then(OsStr::to_str) else {
            return false;
        };
        let stem = String::from(file_stem);
        let ext = extension.as_ref().to_str().unwrap_or("");
        let mut new_name = stem;
        if !ext.is_empty() {
            new_name.push('.');
            new_name.push_str(ext);
        }
        self.set_file_name(new_name);
        true
    }

    /// Consumes the `PathBuf`, yielding its internal [`OsString`] storage.
    #[must_use]
    #[inline]
    pub fn into_os_string(self) -> OsString {
        self.inner
    }

    /// Yields a mutable reference to the underlying [`OsString`].
    #[must_use]
    #[inline]
    pub fn as_mut_os_string(&mut self) -> &mut OsString {
        &mut self.inner
    }

    /// Returns the capacity of the underlying buffer.
    #[must_use]
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Truncates the path to zero length.
    #[inline]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Reserves capacity for at least `additional` more bytes.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }

    /// Reserves the minimum capacity for at least `additional` more bytes.
    #[inline]
    pub fn reserve_exact(&mut self, additional: usize) {
        self.inner.reserve_exact(additional);
    }

    /// Shrinks the capacity of the underlying buffer as much as possible.
    #[inline]
    pub fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit();
    }

    /// Converts this `PathBuf` into a boxed [`Path`].
    #[must_use]
    #[inline]
    pub fn into_boxed_path(self) -> Box<Path> {
        let raw = Box::into_raw(self.inner.into_boxed_os_str());
        // SAFETY: `Path` is `#[repr(transparent)]` over `OsStr`.
        unsafe { Box::from_raw(raw as *mut Path) }
    }
}

impl Deref for PathBuf {
    type Target = Path;

    #[inline]
    fn deref(&self) -> &Path {
        self.as_path()
    }
}

impl Borrow<Path> for PathBuf {
    #[inline]
    fn borrow(&self) -> &Path {
        self.as_path()
    }
}

impl ToOwned for Path {
    type Owned = PathBuf;

    #[inline]
    fn to_owned(&self) -> PathBuf {
        self.to_path_buf()
    }
}

impl AsRef<Path> for Path {
    #[inline]
    fn as_ref(&self) -> &Path {
        self
    }
}

impl AsRef<Path> for PathBuf {
    #[inline]
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl AsRef<Path> for str {
    #[inline]
    fn as_ref(&self) -> &Path {
        Path::new(self)
    }
}

impl AsRef<Path> for String {
    #[inline]
    fn as_ref(&self) -> &Path {
        Path::new(self)
    }
}

impl AsRef<Path> for OsStr {
    #[inline]
    fn as_ref(&self) -> &Path {
        Path::new(self)
    }
}

impl AsRef<Path> for OsString {
    #[inline]
    fn as_ref(&self) -> &Path {
        Path::new(self)
    }
}

impl AsRef<OsStr> for Path {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        self.as_os_str()
    }
}

impl AsRef<OsStr> for PathBuf {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        self.as_os_str()
    }
}

impl<T: ?Sized + AsRef<OsStr>> From<&T> for PathBuf {
    #[inline]
    fn from(s: &T) -> PathBuf {
        PathBuf {
            inner: s.as_ref().to_os_string(),
        }
    }
}

impl From<OsString> for PathBuf {
    #[inline]
    fn from(s: OsString) -> PathBuf {
        PathBuf { inner: s }
    }
}

impl From<String> for PathBuf {
    #[inline]
    fn from(s: String) -> PathBuf {
        PathBuf {
            inner: OsString::from(s),
        }
    }
}

impl From<PathBuf> for OsString {
    #[inline]
    fn from(p: PathBuf) -> OsString {
        p.inner
    }
}

impl FromStr for PathBuf {
    type Err = Infallible;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(PathBuf::from(s))
    }
}

impl<P: AsRef<Path>> FromIterator<P> for PathBuf {
    fn from_iter<I: IntoIterator<Item = P>>(iter: I) -> PathBuf {
        let mut buf = PathBuf::new();
        buf.extend(iter);
        buf
    }
}

impl<P: AsRef<Path>> Extend<P> for PathBuf {
    fn extend<I: IntoIterator<Item = P>>(&mut self, iter: I) {
        for p in iter {
            self.push(p);
        }
    }
}

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

impl fmt::Debug for PathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

impl PartialEq for Path {
    #[inline]
    fn eq(&self, other: &Path) -> bool {
        self.components() == other.components()
    }
}

impl Eq for Path {}

impl PartialOrd for Path {
    #[inline]
    fn partial_cmp(&self, other: &Path) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Path {
    #[inline]
    fn cmp(&self, other: &Path) -> Ordering {
        self.components().cmp(other.components())
    }
}

impl Hash for Path {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for component in self.components() {
            component.hash(state);
        }
    }
}

impl PartialEq for PathBuf {
    #[inline]
    fn eq(&self, other: &PathBuf) -> bool {
        self.as_path() == other.as_path()
    }
}

impl Eq for PathBuf {}

impl PartialOrd for PathBuf {
    #[inline]
    fn partial_cmp(&self, other: &PathBuf) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PathBuf {
    #[inline]
    fn cmp(&self, other: &PathBuf) -> Ordering {
        self.as_path().cmp(other.as_path())
    }
}

impl Hash for PathBuf {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_path().hash(state);
    }
}

impl PartialEq<Path> for PathBuf {
    #[inline]
    fn eq(&self, other: &Path) -> bool {
        self.as_path() == other
    }
}

impl PartialEq<&Path> for PathBuf {
    #[inline]
    fn eq(&self, other: &&Path) -> bool {
        self.as_path() == *other
    }
}

impl PartialEq<PathBuf> for Path {
    #[inline]
    fn eq(&self, other: &PathBuf) -> bool {
        self == other.as_path()
    }
}

impl PartialEq<str> for Path {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self == Path::new(other)
    }
}

impl PartialEq<str> for PathBuf {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_path() == Path::new(other)
    }
}

impl PartialEq<&str> for PathBuf {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_path() == Path::new(*other)
    }
}
