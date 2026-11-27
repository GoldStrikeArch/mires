//! Filesystem stubs returning [`io::ErrorKind::Unsupported`], matching
//! upstream `wasm32-unknown-unknown`.

use crate::ffi::OsString;
use crate::io::{self, Read, Seek, SeekFrom, Write};
use crate::path::{Path, PathBuf};
use crate::string::String;
use crate::time::SystemTime;
use crate::vec::Vec;

fn unsupported<T>() -> io::Result<T> {
    Err(io::Error::unsupported(
        "filesystem operations are not supported on JavaScript targets",
    ))
}

/// An object providing access to an open file (unsupported on JS targets).
#[derive(Debug)]
pub struct File {
    _private: (),
}

impl File {
    /// Attempts to open a file in read-only mode.
    pub fn open<P: AsRef<Path>>(_path: P) -> io::Result<File> {
        unsupported()
    }

    /// Opens a file in write-only mode.
    pub fn create<P: AsRef<Path>>(_path: P) -> io::Result<File> {
        unsupported()
    }

    /// Returns a new [`OpenOptions`] object.
    #[must_use]
    pub fn options() -> OpenOptions {
        OpenOptions::new()
    }

    /// Queries metadata about the underlying file.
    pub fn metadata(&self) -> io::Result<Metadata> {
        unsupported()
    }

    /// Attempts to sync all OS-internal metadata to disk.
    pub fn sync_all(&self) -> io::Result<()> {
        unsupported()
    }

    /// Attempts to sync file data to disk.
    pub fn sync_data(&self) -> io::Result<()> {
        unsupported()
    }

    /// Truncates or extends the underlying file.
    pub fn set_len(&self, _size: u64) -> io::Result<()> {
        unsupported()
    }

    /// Changes the permissions on the underlying file.
    pub fn set_permissions(&self, _perm: Permissions) -> io::Result<()> {
        unsupported()
    }

    /// Creates a new independently owned handle to the underlying file.
    pub fn try_clone(&self) -> io::Result<File> {
        unsupported()
    }
}

impl Read for File {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        unsupported()
    }
}

impl Read for &File {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        unsupported()
    }
}

impl Write for File {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        unsupported()
    }

    fn flush(&mut self) -> io::Result<()> {
        unsupported()
    }
}

impl Write for &File {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        unsupported()
    }

    fn flush(&mut self) -> io::Result<()> {
        unsupported()
    }
}

impl Seek for File {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        unsupported()
    }
}

impl Seek for &File {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        unsupported()
    }
}

/// Options and flags which can be used to configure how a file is opened.
#[derive(Clone, Debug, Default)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
}

impl OpenOptions {
    /// Creates a blank new set of options ready for configuration.
    #[must_use]
    pub fn new() -> OpenOptions {
        OpenOptions::default()
    }

    /// Sets the option for read access.
    pub fn read(&mut self, read: bool) -> &mut Self {
        self.read = read;
        self
    }

    /// Sets the option for write access.
    pub fn write(&mut self, write: bool) -> &mut Self {
        self.write = write;
        self
    }

    /// Sets the option for the append mode.
    pub fn append(&mut self, append: bool) -> &mut Self {
        self.append = append;
        self
    }

    /// Sets the option for truncating a previous file.
    pub fn truncate(&mut self, truncate: bool) -> &mut Self {
        self.truncate = truncate;
        self
    }

    /// Sets the option to create a new file, or open it if it already exists.
    pub fn create(&mut self, create: bool) -> &mut Self {
        self.create = create;
        self
    }

    /// Sets the option to create a new file, failing if it already exists.
    pub fn create_new(&mut self, create_new: bool) -> &mut Self {
        self.create_new = create_new;
        self
    }

    /// Opens a file at `path` with the options specified by `self`.
    pub fn open<P: AsRef<Path>>(&self, _path: P) -> io::Result<File> {
        unsupported()
    }
}

/// Representation of the various permissions on a file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Permissions {
    readonly: bool,
}

impl Permissions {
    /// Returns `true` if these permissions describe a readonly file.
    #[must_use]
    pub fn readonly(&self) -> bool {
        self.readonly
    }

    /// Modifies the readonly flag for this set of permissions.
    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
    }
}

/// A structure representing a type of file with accessors for each file type.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FileType {
    _private: (),
}

impl FileType {
    /// Tests whether this file type represents a directory.
    #[must_use]
    pub fn is_dir(&self) -> bool {
        false
    }

    /// Tests whether this file type represents a regular file.
    #[must_use]
    pub fn is_file(&self) -> bool {
        false
    }

    /// Tests whether this file type represents a symbolic link.
    #[must_use]
    pub fn is_symlink(&self) -> bool {
        false
    }
}

/// Metadata information about a file.
#[derive(Clone, Debug)]
pub struct Metadata {
    _private: (),
}

impl Metadata {
    /// Returns the file type for this metadata.
    #[must_use]
    pub fn file_type(&self) -> FileType {
        FileType { _private: () }
    }

    /// Returns `true` if this metadata is for a directory.
    #[must_use]
    pub fn is_dir(&self) -> bool {
        false
    }

    /// Returns `true` if this metadata is for a regular file.
    #[must_use]
    pub fn is_file(&self) -> bool {
        false
    }

    /// Returns `true` if this metadata is for a symbolic link.
    #[must_use]
    pub fn is_symlink(&self) -> bool {
        false
    }

    /// Returns the size of the file in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        0
    }

    /// Returns `true` if the file has length zero.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        true
    }

    /// Returns the permissions of the file.
    #[must_use]
    pub fn permissions(&self) -> Permissions {
        Permissions { readonly: true }
    }

    /// Returns the last modification time.
    pub fn modified(&self) -> io::Result<SystemTime> {
        unsupported()
    }

    /// Returns the last access time.
    pub fn accessed(&self) -> io::Result<SystemTime> {
        unsupported()
    }

    /// Returns the creation time.
    pub fn created(&self) -> io::Result<SystemTime> {
        unsupported()
    }
}

/// Entries returned by the [`ReadDir`] iterator.
#[derive(Debug)]
pub struct DirEntry {
    _private: (),
}

impl DirEntry {
    /// Returns the full path to the file that this entry represents.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        PathBuf::new()
    }

    /// Returns the metadata for the file that this entry points at.
    pub fn metadata(&self) -> io::Result<Metadata> {
        unsupported()
    }

    /// Returns the file type for the file that this entry points at.
    pub fn file_type(&self) -> io::Result<FileType> {
        unsupported()
    }

    /// Returns the file name of this directory entry without any leading path component(s).
    #[must_use]
    pub fn file_name(&self) -> OsString {
        OsString::new()
    }
}

/// Iterator over the entries in a directory.
#[derive(Debug)]
pub struct ReadDir {
    _private: (),
}

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;

    fn next(&mut self) -> Option<io::Result<DirEntry>> {
        None
    }
}

/// A builder used to create directories in various manners.
#[derive(Debug, Default)]
pub struct DirBuilder {
    recursive: bool,
}

impl DirBuilder {
    /// Creates a new set of options with default mode/security settings.
    #[must_use]
    pub fn new() -> DirBuilder {
        DirBuilder { recursive: false }
    }

    /// Indicates that directories should be created recursively.
    pub fn recursive(&mut self, recursive: bool) -> &mut Self {
        self.recursive = recursive;
        self
    }

    /// Creates the specified directory with the options configured in this builder.
    pub fn create<P: AsRef<Path>>(&self, _path: P) -> io::Result<()> {
        unsupported()
    }
}

/// Reads the entire contents of a file into a bytes vector (`Unsupported`).
pub fn read<P: AsRef<Path>>(_path: P) -> io::Result<Vec<u8>> {
    unsupported()
}

/// Reads the entire contents of a file into a string (`Unsupported`).
pub fn read_to_string<P: AsRef<Path>>(_path: P) -> io::Result<String> {
    unsupported()
}

/// Writes a slice as the entire contents of a file (`Unsupported`).
pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(_path: P, _contents: C) -> io::Result<()> {
    unsupported()
}

/// Creates a new, empty directory at the provided path (`Unsupported`).
pub fn create_dir<P: AsRef<Path>>(_path: P) -> io::Result<()> {
    unsupported()
}

/// Recursively creates a directory and all of its missing parent components (`Unsupported`).
pub fn create_dir_all<P: AsRef<Path>>(_path: P) -> io::Result<()> {
    unsupported()
}

/// Removes a file from the filesystem (`Unsupported`).
pub fn remove_file<P: AsRef<Path>>(_path: P) -> io::Result<()> {
    unsupported()
}

/// Removes an empty directory (`Unsupported`).
pub fn remove_dir<P: AsRef<Path>>(_path: P) -> io::Result<()> {
    unsupported()
}

/// Removes a directory and all of its contents (`Unsupported`).
pub fn remove_dir_all<P: AsRef<Path>>(_path: P) -> io::Result<()> {
    unsupported()
}

/// Renames a file or directory to a new name (`Unsupported`).
pub fn rename<P: AsRef<Path>, Q: AsRef<Path>>(_from: P, _to: Q) -> io::Result<()> {
    unsupported()
}

/// Copies the contents of one file to another (`Unsupported`).
pub fn copy<P: AsRef<Path>, Q: AsRef<Path>>(_from: P, _to: Q) -> io::Result<u64> {
    unsupported()
}

/// Queries metadata about a path (`Unsupported`).
pub fn metadata<P: AsRef<Path>>(_path: P) -> io::Result<Metadata> {
    unsupported()
}

/// Queries metadata about a path without following symlinks (`Unsupported`).
pub fn symlink_metadata<P: AsRef<Path>>(_path: P) -> io::Result<Metadata> {
    unsupported()
}

/// Returns an iterator over the entries within a directory (`Unsupported`).
pub fn read_dir<P: AsRef<Path>>(_path: P) -> io::Result<ReadDir> {
    unsupported()
}

/// Returns the canonical, absolute form of a path (`Unsupported`).
pub fn canonicalize<P: AsRef<Path>>(_path: P) -> io::Result<PathBuf> {
    unsupported()
}

/// Creates a new hard link on the filesystem (`Unsupported`).
pub fn hard_link<P: AsRef<Path>, Q: AsRef<Path>>(_original: P, _link: Q) -> io::Result<()> {
    unsupported()
}

/// Reads a symbolic link (`Unsupported`).
pub fn read_link<P: AsRef<Path>>(_path: P) -> io::Result<PathBuf> {
    unsupported()
}

/// Changes the permissions found on a file or a directory (`Unsupported`).
pub fn set_permissions<P: AsRef<Path>>(_path: P, _perm: Permissions) -> io::Result<()> {
    unsupported()
}

/// Returns `Ok(true)` if the path exists (`Unsupported`).
pub fn exists<P: AsRef<Path>>(_path: P) -> io::Result<bool> {
    unsupported()
}
