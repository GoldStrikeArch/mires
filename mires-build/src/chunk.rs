use std::path::{Path, PathBuf};

use crate::{BuildError, Result};

/// The `-Cllvm-args` option naming one chunk, `js-chunk=<name>:<entry>`.
///
/// Repeatable: one occurrence per chunk, in the order the chunks were
/// requested.
pub const CHUNK_ARG: &str = "js-chunk";
/// The `-Cllvm-args` option naming the shared chunk, `js-chunk-shared=<name>`.
pub const SHARED_CHUNK_ARG: &str = "js-chunk-shared";
/// The prefix an island's exported entry point is named with.
pub const ISLAND_ENTRY_PREFIX: &str = "__island_";
/// Default [`dir`](Chunking::dir): the directory inside `OUT_DIR` the chunks
/// are published to.
pub const DEFAULT_CHUNK_DIR: &str = "chunks";
/// Default [`shared`](Chunking::shared) chunk name.
pub const DEFAULT_SHARED_NAME: &str = "shared";
/// Default [`specifier`](Chunking::specifier) prefix.
pub const DEFAULT_SPECIFIER_PREFIX: &str = "mires-island/";

/// One emitted JavaScript file, named by the chunk it holds.
///
/// A program that was not split is one chunk named after its output file, so
/// the same code reads a split program and an unsplit one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The name the chunk is requested, emitted, and served under.
    pub name: String,
    /// The published file inside `OUT_DIR`.
    pub path: PathBuf,
    /// The source map beside it, when
    /// [`source_map`](crate::BuildConfig::source_map) is on.
    pub map: Option<PathBuf>,
}

impl Chunk {
    /// The chunk's published file name, which is also the last segment of the
    /// URL it is served from.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
    }
}

/// How a program is split into files, and how the pieces are named, published,
/// and served.
///
/// One chunk per island, plus a shared chunk holding what two or more islands
/// both reach. Hand it to
/// [`BuildConfig::chunks`](crate::BuildConfig::chunks):
///
/// ```rust,no_run
/// use mires_build::{BuildConfig, Chunking};
///
/// BuildConfig::new("client-island/island.rs")
///     .output_name("islands.js")
///     .js_extern(true)
///     .view_abi(true)
///     .crate_arg("js-modules=esm")
///     .chunks(
///         Chunking::new()
///             .island("counter")
///             .island("search")
///             .url_base("/demo/chunks/")
///             .import("mires-dom", "/demo/mires-dom.js"),
///     )
///     .render()
///     .unwrap();
/// ```
///
/// # What the backend is asked for
///
/// The request reaches the backend as [`args`](Self::args), one
/// [`CHUNK_ARG`] per chunk plus [`SHARED_CHUNK_ARG`]. A backend that honours
/// them writes one file per chunk beside the output path, named after the
/// chunk, plus the shared file when there is anything to share. These are link
/// step options, like `js-modules`, so they cost no sysroot rebuild.
///
/// A backend that emits one file for the whole crate leaves those files
/// absent, and every requested chunk then names that one file: the served URLs
/// collapse onto it, the import map still resolves, and the page still asks
/// for an island by name. That is what makes the request safe to write before
/// the backend can split, and it is why a chunk name is not a file name until
/// the compile has run.
#[derive(Debug, Clone)]
pub struct Chunking {
    entries: Vec<(String, String)>,
    shared: String,
    dir: String,
    specifier: String,
    url_base: Option<String>,
    imports: Vec<(String, String)>,
}

impl Chunking {
    /// A request for no chunks, ready to have them added.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            shared: DEFAULT_SHARED_NAME.to_owned(),
            dir: DEFAULT_CHUNK_DIR.to_owned(),
            specifier: DEFAULT_SPECIFIER_PREFIX.to_owned(),
            url_base: None,
            imports: Vec::new(),
        }
    }

    /// Ask for a chunk holding the island called `name`, whose entry point is
    /// the export [`ISLAND_ENTRY_PREFIX`] and that name.
    ///
    /// Calls accumulate, in order.
    #[must_use]
    pub fn island(self, name: impl Into<String>) -> Self {
        let name = name.into();
        let entry = format!("{ISLAND_ENTRY_PREFIX}{name}");
        self.entry(name, entry)
    }

    /// Ask for a chunk called `name` holding everything the export `entry`
    /// reaches.
    ///
    /// Calls accumulate, in order. [`island`](Self::island) is this with the
    /// export name spelled for you.
    #[must_use]
    pub fn entry(mut self, name: impl Into<String>, entry: impl Into<String>) -> Self {
        self.entries.push((name.into(), entry.into()));
        self
    }

    /// Name of the chunk holding what two or more chunks both reach. Defaults
    /// to [`DEFAULT_SHARED_NAME`].
    #[must_use]
    pub fn shared(mut self, name: impl Into<String>) -> Self {
        self.shared = name.into();
        self
    }

    /// Directory inside `OUT_DIR` the chunks are published to. Defaults to
    /// [`DEFAULT_CHUNK_DIR`].
    ///
    /// Every chunk lands here, so one route serving this directory serves the
    /// whole program.
    #[must_use]
    pub fn dir(mut self, dir: impl Into<String>) -> Self {
        self.dir = dir.into();
        self
    }

    /// Prefix of the module specifier a chunk is imported by. Defaults to
    /// [`DEFAULT_SPECIFIER_PREFIX`].
    ///
    /// The specifier is this and the chunk name. It is bare, so the page's
    /// import map is what resolves it, which is what lets a chunk move without
    /// the code that imports it changing.
    #[must_use]
    pub fn specifier(mut self, prefix: impl Into<String>) -> Self {
        self.specifier = prefix.into();
        self
    }

    /// The URL the chunk directory is served from, which turns on the import
    /// map.
    ///
    /// A chunk's URL is this and its published file name, so the base ends in
    /// a slash. Without it there is no import map to write, since nothing says
    /// where the chunks will be.
    #[must_use]
    pub fn url_base(mut self, base: impl Into<String>) -> Self {
        self.url_base = Some(base.into());
        self
    }

    /// Add an entry to the import map that is not a chunk, such as the runtime
    /// the chunks import.
    ///
    /// Calls accumulate, and these come before the chunks, so the map reads
    /// with the page's own specifiers first.
    #[must_use]
    pub fn import(mut self, specifier: impl Into<String>, url: impl Into<String>) -> Self {
        self.imports.push((specifier.into(), url.into()));
        self
    }

    /// Whether no chunk was asked for, in which case the program is emitted as
    /// one file.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The requested chunk names, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(name, _)| name.as_str())
    }

    /// The name of the shared chunk.
    #[must_use]
    pub fn shared_name(&self) -> &str {
        &self.shared
    }

    /// The directory inside `OUT_DIR` the chunks are published to.
    #[must_use]
    pub fn directory(&self) -> &str {
        &self.dir
    }

    /// The backend options that ask for this split: one [`CHUNK_ARG`] per
    /// chunk, then [`SHARED_CHUNK_ARG`].
    ///
    /// Empty when no chunk was asked for, so a program that is not split is
    /// compiled with exactly the options it was before.
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        if self.entries.is_empty() {
            return Vec::new();
        }
        let mut args: Vec<String> = self
            .entries
            .iter()
            .map(|(name, entry)| format!("{CHUNK_ARG}={name}:{entry}"))
            .collect();
        args.push(format!("{SHARED_CHUNK_ARG}={}", self.shared));
        args
    }

    /// The file the backend writes the chunk called `name` to, beside the
    /// output path.
    #[must_use]
    pub fn emitted(&self, staging: &Path, name: &str) -> PathBuf {
        staging.join(format!("{name}.js"))
    }

    /// The module specifier the chunk called `name` is imported by.
    #[must_use]
    pub fn specifier_for(&self, name: &str) -> String {
        format!("{}{name}", self.specifier)
    }

    /// The import map resolving every chunk's specifier to the URL it is
    /// served from, plus whatever [`import`](Self::import) added.
    ///
    /// This is a whole import map, ready to be the body of a
    /// `<script type="importmap">`. `None` when no
    /// [`url_base`](Self::url_base) says where the chunks will be.
    #[must_use]
    pub fn import_map(&self, chunks: &[Chunk]) -> Option<String> {
        let base = self.url_base.as_deref()?;
        let entries = self
            .imports
            .iter()
            .map(|(specifier, url)| (specifier.clone(), url.clone()));
        let chunks = chunks.iter().map(|chunk| {
            (
                self.specifier_for(&chunk.name),
                format!("{base}{}", chunk.file_name()),
            )
        });

        let mut json = String::from(r#"{"imports":{"#);
        for (index, (specifier, url)) in entries.chain(chunks).enumerate() {
            if index > 0 {
                json.push(',');
            }
            push_string(&mut json, &specifier);
            json.push(':');
            push_string(&mut json, &url);
        }
        json.push_str("}}");
        Some(json)
    }

    /// Check every name and entry point can be spelled in a backend option and
    /// in a file name.
    ///
    /// # Errors
    ///
    /// Returns `Err` naming the first name that cannot.
    pub(crate) fn check(&self) -> Result {
        for (name, entry) in &self.entries {
            check_name(name)?;
            check_name(entry)?;
        }
        check_name(&self.shared)
    }
}

impl Default for Chunking {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether `name` can be both a `js-chunk` value and a file name.
///
/// The option is split on `=`, `:` and whitespace on the way to the backend,
/// and the name becomes a file name and a URL segment on the way out, so the
/// safe set is the one every layer leaves alone.
fn check_name(name: &str) -> Result {
    let usable = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '$'));
    if usable {
        return Ok(());
    }
    Err(BuildError::ChunkName {
        name: name.to_owned(),
    })
}

/// Append `value` to `json` as a JSON string.
fn push_string(json: &mut String, value: &str) {
    json.push('"');
    for character in value.chars() {
        match character {
            '"' => json.push_str("\\\""),
            '\\' => json.push_str("\\\\"),
            '\n' => json.push_str("\\n"),
            '\r' => json.push_str("\\r"),
            '\t' => json.push_str("\\t"),
            control if control.is_control() => {
                json.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => json.push(other),
        }
    }
    json.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(name: &str, file: &str) -> Chunk {
        Chunk {
            name: name.to_owned(),
            path: PathBuf::from("/out/chunks").join(file),
            map: None,
        }
    }

    #[test]
    fn no_chunks_asks_the_backend_for_nothing() {
        let chunking = Chunking::new();

        assert!(chunking.is_empty());
        assert!(chunking.args().is_empty());
        assert_eq!(chunking.names().count(), 0);
    }

    #[test]
    fn an_island_chunk_is_rooted_at_its_exported_entry_point() {
        let chunking = Chunking::new().island("counter").island("search");

        assert_eq!(
            chunking.args(),
            [
                "js-chunk=counter:__island_counter",
                "js-chunk=search:__island_search",
                "js-chunk-shared=shared",
            ]
        );
    }

    #[test]
    fn an_entry_point_can_be_named_outright() {
        let chunking = Chunking::new().entry("boot", "start").shared("common");

        assert_eq!(
            chunking.args(),
            ["js-chunk=boot:start", "js-chunk-shared=common"]
        );
        assert_eq!(chunking.shared_name(), "common");
    }

    #[test]
    fn chunk_names_are_kept_in_the_order_they_were_asked_for() {
        let chunking = Chunking::new().island("counter").island("search");

        let names: Vec<&str> = chunking.names().collect();

        assert_eq!(names, ["counter", "search"]);
    }

    #[test]
    fn a_chunk_is_emitted_beside_the_output_path() {
        let chunking = Chunking::new().island("counter");

        assert_eq!(
            chunking.emitted(Path::new("/out/.mires-build"), "counter"),
            Path::new("/out/.mires-build/counter.js")
        );
    }

    #[test]
    fn the_defaults_name_the_directory_the_specifier_and_the_shared_chunk() {
        let chunking = Chunking::new();

        assert_eq!(chunking.directory(), "chunks");
        assert_eq!(chunking.shared_name(), "shared");
        assert_eq!(chunking.specifier_for("counter"), "mires-island/counter");
    }

    #[test]
    fn the_specifier_prefix_and_directory_can_be_replaced() {
        let chunking = Chunking::new().specifier("app:").dir("islands");

        assert_eq!(chunking.specifier_for("counter"), "app:counter");
        assert_eq!(chunking.directory(), "islands");
    }

    #[test]
    fn there_is_no_import_map_until_a_url_says_where_the_chunks_are() {
        let chunking = Chunking::new().island("counter");

        assert_eq!(chunking.import_map(&[chunk("counter", "counter.js")]), None);
    }

    #[test]
    fn the_import_map_resolves_every_chunk_to_its_served_url() {
        let chunking = Chunking::new()
            .island("counter")
            .island("search")
            .url_base("/demo/chunks/");

        let map = chunking
            .import_map(&[
                chunk("counter", "counter.js"),
                chunk("search", "search.js"),
                chunk("shared", "shared.js"),
            ])
            .unwrap();

        assert_eq!(
            map,
            concat!(
                r#"{"imports":{"mires-island/counter":"/demo/chunks/counter.js","#,
                r#""mires-island/search":"/demo/chunks/search.js","#,
                r#""mires-island/shared":"/demo/chunks/shared.js"}}"#,
            )
        );
    }

    #[test]
    fn the_pages_own_specifiers_come_before_the_chunks() {
        let chunking = Chunking::new()
            .island("counter")
            .url_base("/demo/chunks/")
            .import("mires-dom", "/demo/mires-dom.js");

        let map = chunking
            .import_map(&[chunk("counter", "counter.js")])
            .unwrap();

        assert_eq!(
            map,
            concat!(
                r#"{"imports":{"mires-dom":"/demo/mires-dom.js","#,
                r#""mires-island/counter":"/demo/chunks/counter.js"}}"#,
            )
        );
    }

    #[test]
    fn an_unsplit_program_maps_every_chunk_onto_the_one_file() {
        let chunking = Chunking::new()
            .island("counter")
            .island("search")
            .url_base("/demo/chunks/");

        let map = chunking
            .import_map(&[
                chunk("counter", "islands.js"),
                chunk("search", "islands.js"),
            ])
            .unwrap();

        assert_eq!(
            map,
            concat!(
                r#"{"imports":{"mires-island/counter":"/demo/chunks/islands.js","#,
                r#""mires-island/search":"/demo/chunks/islands.js"}}"#,
            )
        );
    }

    #[test]
    fn a_url_with_a_quote_in_it_stays_inside_its_json_string() {
        let chunking = Chunking::new()
            .url_base("/demo/")
            .import(r#"a"b"#, "/x\\y\n");

        let map = chunking.import_map(&[]).unwrap();

        assert_eq!(map, r#"{"imports":{"a\"b":"/x\\y\n"}}"#);
    }

    #[test]
    fn a_chunks_file_name_is_the_last_segment_of_its_url() {
        assert_eq!(chunk("counter", "counter.js").file_name(), "counter.js");
    }

    #[test]
    fn a_usable_name_is_letters_digits_and_a_few_marks() {
        for name in ["counter", "a-b", "a_b", "__island_x", "x9", "$x"] {
            assert!(check_name(name).is_ok(), "{name} should be usable");
        }
    }

    #[test]
    fn a_name_that_would_not_survive_the_option_is_refused() {
        for name in ["", "a:b", "a=b", "a b", "a/b", "a.b", "hö"] {
            let error = check_name(name).unwrap_err();
            assert!(
                matches!(&error, BuildError::ChunkName { name: bad } if bad == name),
                "{name} should be refused, got {error}"
            );
        }
    }

    #[test]
    fn checking_reports_the_first_unusable_name() {
        let error = Chunking::new()
            .island("counter")
            .entry("a b", "start")
            .check()
            .unwrap_err();

        assert!(matches!(error, BuildError::ChunkName { name } if name == "a b"));
    }

    #[test]
    fn the_shared_chunks_name_is_checked_too() {
        let error = Chunking::new()
            .island("counter")
            .shared("a b")
            .check()
            .unwrap_err();

        assert!(matches!(error, BuildError::ChunkName { name } if name == "a b"));
    }
}
