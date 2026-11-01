// What the build says about a bundle without changing it: warnings only, none of which affects
// what gets built or embedded.

use std::path::{Path, PathBuf};

use super::is_one_line;
use crate::Error;

/// Which of the names a manifest reported are no longer files beside it.
///
/// Empty on any build where a bundler has just run, and that is the point: anything here was
/// taken out afterwards, by something that was not this.
fn missing_from<'a>(output: &Path, names: &'a [String]) -> Vec<&'a String> {
    names
        .iter()
        .filter(|name| !output.join(name).is_file())
        .collect()
}

/// What the entry document puts in front of the names the bundler emitted, when that is anything
/// other than a bare `/`.
///
/// The names come from the manifest and are what the bundle is keyed by; the document is what a
/// browser is about to read. Whatever sits between the opening quote and the name is a prefix that
/// has to exist on the router too, and usually does not.
fn asset_prefix<'a>(document: &'a str, names: &[String]) -> Option<&'a str> {
    for name in names {
        let Some(at) = document.find(name.as_str()) else {
            continue;
        };

        // Back to whichever quote opened the attribute, leaving exactly what the bundler wrote in
        // front of the name.
        let prefix = document[..at]
            .rsplit(['"', '\''])
            .next()
            .unwrap_or_default();

        if prefix == "/" || !is_one_line(prefix) {
            continue;
        }

        return Some(prefix);
    }

    None
}

/// Every file under the bundle, with its size.
///
/// Plain recursion rather than the walker the sources use. That one honours ignore files, and a
/// bundle's directory is ignored by every frontend that has one — it would come back empty.
///
/// `DirEntry::file_type` does not follow links, so a directory reached through one is not
/// descended into. That leaves a cycle unable to form, which matters more here than counting a
/// linked file would.
fn bundled(output: &Path) -> Result<Vec<(PathBuf, u64)>, Error> {
    let mut found = Vec::new();
    let mut pending = vec![output.to_path_buf()];
    // Links are followed, so a directory that leads back up its own tree would be walked until
    // this ran out of memory. Each one is entered once, by wherever it really is.
    let mut entered = std::collections::HashSet::new();

    while let Some(directory) = pending.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            // Nothing to describe. `run` has already established there is a bundle by the time
            // this is asked, so this is a directory that went away underneath us.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(Error::io(&directory, error)),
        };

        for entry in entries {
            let entry = entry.map_err(|error| Error::io(&directory, error))?;
            // Followed, because `rust_embed` follows them: a link is a file to whatever embeds
            // this, so a walk that skipped one would describe a bundle nobody has — and, since
            // the store is filled from this, embed one nobody has either.
            let found_at = match std::fs::metadata(entry.path()) {
                Ok(metadata) => metadata,
                // A link to nothing, or something taken away mid-walk. Nothing to describe and
                // nothing to embed.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(Error::io(&entry.path(), error)),
            };

            if found_at.is_dir() {
                // By where it really is, so a link and the directory it points at are one place.
                let once = std::fs::canonicalize(entry.path()).unwrap_or_else(|_| entry.path());

                if entered.insert(once) {
                    pending.push(entry.path());
                }
            } else if found_at.is_file() {
                found.push((entry.path(), found_at.len()));
            }
        }
    }

    Ok(found)
}

/// How much of the bundle is source maps, and how much is compressed copies of its own files.
/// What the bundler wrote that nothing will ever ask for.
#[derive(Debug, Default, PartialEq, Eq)]
struct Baggage {
    source_maps: Weight,
    precompressed: Weight,
}

impl Baggage {
    fn of(files: &[(PathBuf, u64)]) -> Self {
        let names: std::collections::HashSet<&Path> =
            files.iter().map(|(path, _)| path.as_path()).collect();

        let mut found = Self::default();

        for (path, size) in files {
            match path.extension().and_then(std::ffi::OsStr::to_str) {
                Some("map") => found.source_maps.add(*size),
                // Only when the thing it compresses is here too. A lone `.gz` is an asset the
                // frontend fetches and unpacks itself, which nothing here should discourage.
                Some("br" | "gz") if names.contains(path.with_extension("").as_path()) => {
                    found.precompressed.add(*size);
                }
                _ => {}
            }
        }

        found
    }
}

/// What the bundle is carrying that nobody asked to ship, said so it reads the same whether there
/// is one of them or forty.
///
/// One source map is what a small application produces — a single entry chunk has a single map —
/// so the singular is the ordinary case rather than the corner. Both of these are kept apart from
/// printing them so that a test can read either count.
fn source_map_warning(maps: &Weight) -> Option<String> {
    (maps.files > 0).then(|| {
        format!(
            "the bundle holds {} source map{} ({}). Each one is embedded like everything else, \
             and a browser with developer tools open asks for it — which hands over the original \
             source and the paths it was built from. Turn source maps off for the production \
             build, or keep them out of the binary with an `#[exclude = \"*.map\"]` beside the \
             `#[folder]` on the embed.",
            maps.files,
            plural(maps.files, "", "s"),
            sized(maps.bytes)
        )
    })
}

fn precompressed_warning(precompressed: &Weight) -> Option<String> {
    (precompressed.files > 0).then(|| {
        format!(
            "the bundle holds {} precompressed {} ({}), each one a `.br` or `.gz` beside the file \
             it compresses. They are left out of the binary — the bundle is compressed on its way \
             in, at a quality a bundler plugin does not reach — so this is work being done twice. \
             Turn that plugin off.",
            precompressed.files,
            plural(precompressed.files, "copy", "copies"),
            sized(precompressed.bytes)
        )
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Weight {
    files: usize,
    bytes: u64,
}

impl Weight {
    fn add(&mut self, bytes: u64) {
        self.files += 1;
        self.bytes += bytes;
    }
}

/// A size a person can judge at a glance, which is all these warnings need. Integer arithmetic
/// throughout, so there is no rounding to reason about.
fn sized(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * KB;

    if bytes >= MB {
        format!("{}.{} MB", bytes / MB, (bytes % MB) * 10 / MB)
    } else {
        format!("{} KB", bytes.div_ceil(KB))
    }
}

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

pub(super) fn unreported_unhashed_name_warning(name: &str) -> String {
    format!(
        "`Manifest::unhashed_name` declares {name:?}, which the manifest does not report, so it \
         has no effect. Use the path the manifest reports, relative to the output directory."
    )
}

/// Why one of the names a bundler emitted is not published as cacheable. The two characters that
/// put a name here go wrong differently — a separator splits the name in two, a line ending ends
/// the instruction — and the message says which.
pub(super) fn unpublishable_warning(name: &str) -> String {
    let because = if name.contains(['\n', '\r']) {
        "a build script says everything in single lines, and the line ending in this one would \
         finish the instruction early and leave cargo reading the rest of the name as another one"
            .to_owned()
    } else {
        format!(
            "the names travel to the compiler as one line with {:?} between them, so this one \
             would arrive as two names and match nothing",
            crate::SEPARATOR
        )
    };

    format!(
        "{name:?} cannot be reported as cacheable: {because}. It will be revalidated on every \
         load until it is renamed."
    )
}

/// What the manifest named that is not there, counted the way someone reads it.
///
/// Kept apart from printing it so a test can read what it says. The counting is the whole reason:
/// one file reads badly as a ratio of one to one, and every file reads worse as a ratio of a
/// number to itself — which is not the unlikely case here but the likely one, because what puts a
/// build in this position is usually a second build running over it and taking the lot.
fn missing_warning(output: &Path, names: &[String]) -> Option<String> {
    let gone = missing_from(output, names);
    let first = gone.first()?;

    // Escaped, for the reason its two neighbours are: these names come straight from the manifest,
    // before anything has set aside the ones that cannot be said in a line of build script output.
    // A name holding a line ending would end this warning halfway and leave cargo reading the rest
    // of it as an instruction.

    let counted = if gone.len() == 1 {
        format!("the manifest names {first:?}, which is not in the bundle")
    } else if gone.len() == names.len() {
        format!(
            "none of the {} files the manifest names are in the bundle — {first:?} for one",
            names.len()
        )
    } else {
        format!(
            "{} of the {} files the manifest names are not in the bundle — {first:?} for one",
            gone.len(),
            names.len()
        )
    };

    Some(counted)
}

/// Three situations with opposite advice: an absolute path is a frontend mounted somewhere other
/// than the root, a relative one resolves against whatever route the browser is on, and an
/// absolute URL is a bundle on a CDN — a real way to deploy, not something to correct.
fn prefix_warning(prefix: &str) -> String {
    // A scheme, or the `//` that borrows the page's own. Either way the browser leaves.
    if prefix.starts_with("//") || prefix.contains("://") {
        return format!(
            "the entry document asks for its assets from {prefix:?}, which is somewhere else \
             altogether. A browser fetches them from there and never from this binary, so every \
             one of them is carried in it and never served. That is a way to deploy on purpose — \
             the bundle on a CDN and the document from here — and if it was not the intention, \
             the bundler's `base` is where it was decided."
        );
    }

    if let Some(mount) = prefix.strip_suffix('/').filter(|at| at.starts_with('/')) {
        return format!(
            "the entry document asks for its assets under {prefix:?} and they are served under \
             \"/\". That prefix is the bundler's `base` — `base` in a Vite config — and it has to \
             be the path the frontend is mounted at. Mount it there with \
             `nest_service({mount:?}, ..)`, or take the setting out. As it stands the page loads \
             and nothing on it does."
        );
    }

    format!(
        "the entry document asks for its assets under {prefix:?}, which is relative. A browser \
         resolves that against whatever route it is on, so the first client-side route a visitor \
         lands on asks for the bundle somewhere it has never been. The bundler's `base` has to be \
         an absolute path — `/` if the frontend is mounted at the root."
    )
}

/// What the bundler wrote that the binary would rather not carry. Neither kind is an error —
/// either can be wanted — but both are invisible from the Rust side, where the embed takes
/// the output directory whole.
/// The files the bundler left in the output directory, walked once.
///
/// Every warning below reads the same directory, and the walk is the expensive part of asking.
pub(super) struct Bundle {
    output: PathBuf,
    files: Vec<(PathBuf, u64)>,
}

impl Bundle {
    /// Everything the walk found, as absolute paths.
    pub(super) fn files(&self) -> Vec<PathBuf> {
        self.files.iter().map(|(path, _)| path.clone()).collect()
    }

    pub(super) fn read(output: &Path) -> Result<Self, Error> {
        Ok(Self {
            output: output.to_path_buf(),
            files: bundled(output)?,
        })
    }

    /// What the bundler wrote that the binary would rather not carry. Neither kind is an error —
    /// either can be wanted — but both are invisible from the Rust side, where the embed takes
    /// the output directory whole.
    pub(super) fn warn_about_baggage(&self) {
        let baggage = Baggage::of(&self.files);

        for warning in [
            source_map_warning(&baggage.source_maps),
            precompressed_warning(&baggage.precompressed),
        ]
        .into_iter()
        .flatten()
        {
            println!("cargo::warning={warning}");
        }
    }

    /// Whether everything the manifest reported is still in the bundle. It stops being so when
    /// something removes a file afterwards — most often a second build script bundling over this
    /// one — and nothing else here would notice: the sources have not changed and the entry
    /// document is still there.
    pub(super) fn warn_about_missing(&self, names: &[String]) {
        let Some(counted) = missing_warning(&self.output, names) else {
            return;
        };

        println!(
            "cargo::warning={counted}. That is something removing files after the bundler wrote \
             them, most likely a second build running over this one. Building again will not put \
             them back: the sources have not changed and the entry document is still there, so \
             there is nothing here that looks out of date. Delete {} and build again.",
            self.output.display()
        );
    }
}

/// A bundler told to write its asset URLs under a prefix — `base` in a Vite config — is
/// invisible from here: the manifest still names `assets/index-D4t.js` and the document still
/// loads. Nothing on it does, because every URL it names carries a prefix the router never
/// heard of, and the symptom is a blank page a long way from the setting that caused it.
pub(super) fn warn_about_asset_prefix(entry: &Path, names: &[String]) {
    // A bundler that emits something other than a document is not doing anything wrong.
    let Ok(document) = std::fs::read_to_string(entry) else {
        return;
    };

    let Some(prefix) = asset_prefix(&document, names) else {
        return;
    };

    println!("cargo::warning={}", prefix_warning(prefix));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TempDir, temp_dir};

    fn write_files_of_sizes(files: &[(&str, usize)]) -> TempDir {
        let directory = temp_dir();

        for (name, size) in files {
            directory.write_file(name, "x".repeat(*size));
        }

        directory
    }

    #[test]
    fn bundled_finds_files_at_any_depth() {
        let output = write_files_of_sizes(&[
            ("index.html", 10),
            ("assets/index-A1.js", 20),
            ("assets/deep/nested-B2.css", 30),
        ]);

        let mut files = bundled(output.path()).unwrap();

        files.sort();
        assert_eq!(
            files,
            [
                (output.path().join("assets/deep/nested-B2.css"), 30),
                (output.path().join("assets/index-A1.js"), 20),
                (output.path().join("index.html"), 10),
            ]
        );
    }

    #[test]
    fn bundled_missing_directory_returns_empty() {
        let directory = temp_dir();

        let files = bundled(&directory.path().join("never-written")).unwrap();

        assert_eq!(files, Vec::new());
    }

    #[test]
    fn baggage_of_counts_source_maps_and_copies_beside_their_original() {
        let output = write_files_of_sizes(&[
            ("index.html", 500),
            ("assets/index-A1.js", 900),
            ("assets/index-A1.js.map", 4096),
            ("assets/index-A1.js.gz", 300),
            ("assets/index-A1.js.br", 250),
            ("assets/style-B2.css", 50),
            ("assets/style-B2.css.map", 2048),
            ("assets/dictionary-C3.bin.gz", 5000),
        ]);
        let files = bundled(output.path()).unwrap();

        let baggage = Baggage::of(&files);

        assert_eq!(
            baggage,
            Baggage {
                source_maps: Weight {
                    files: 2,
                    bytes: 6144
                },
                precompressed: Weight {
                    files: 2,
                    bytes: 550
                },
            }
        );
    }

    #[test]
    fn asset_prefix_returns_what_the_document_puts_before_the_name() {
        let names = ["assets/index-D4t.js".to_owned()];

        for (document, prefix) in [
            (
                r#"<script type="module" src="/app/assets/index-D4t.js"></script>"#,
                "/app/",
            ),
            (
                r#"<script type="module" src="./assets/index-D4t.js"></script>"#,
                "./",
            ),
        ] {
            assert_eq!(asset_prefix(document, &names), Some(prefix), "{document}");
        }
    }

    #[test]
    fn asset_prefix_returns_none_when_there_is_no_prefix_to_report() {
        let document = r#"<script type="module" src="/assets/index-D4t.js"></script>"#;

        for (case, document, name) in [
            ("root", document, "assets/index-D4t.js"),
            (
                "name not in the document",
                document,
                "assets/never-referenced-X1.css",
            ),
            (
                "prefix with a line ending",
                "<script src=\"/app\nassets/index-D4t.js\"></script>",
                "assets/index-D4t.js",
            ),
        ] {
            assert_eq!(asset_prefix(document, &[name.to_owned()]), None, "{case}");
        }
    }

    #[test]
    fn prefix_warning_tells_a_url_a_mount_and_a_relative_path_apart() {
        for (prefix, advice) in [
            ("https://cdn.example.com/", "somewhere else altogether"),
            ("//cdn.example.com/", "somewhere else altogether"),
            ("/app/", r#"nest_service("/app", ..)"#),
            ("./", "which is relative"),
        ] {
            let warning = prefix_warning(prefix);

            assert!(warning.contains(advice), "{warning}");
        }
    }

    #[test]
    fn every_warning_escapes_a_name_that_would_end_the_line() {
        const NAME_WITH_NEWLINE_AND_COMMA: &str = "assets/two\nlines,and-a-comma.js";
        let output = temp_dir();
        let names = [NAME_WITH_NEWLINE_AND_COMMA.to_owned()];

        let warnings = [
            missing_warning(output.path(), &names),
            Some(unpublishable_warning(NAME_WITH_NEWLINE_AND_COMMA)),
            Some(unreported_unhashed_name_warning(
                NAME_WITH_NEWLINE_AND_COMMA,
            )),
            Some(prefix_warning("/two\nlines/")),
        ];

        for warning in warnings {
            let warning = warning.expect("each warning has something to report");

            assert!(
                !warning.chars().any(char::is_control),
                "this reaches cargo as more than one line: {warning:?}"
            );
        }
    }

    #[test]
    fn missing_warning_counts_the_names_the_bundle_lost() {
        let names = ["a.js", "b.js", "c.js"].map(str::to_owned);

        for (present, counted) in [
            (&["a.js", "b.js", "c.js"][..], None),
            (
                &["b.js", "c.js"][..],
                Some(r#"the manifest names "a.js", which is not in the bundle"#),
            ),
            (
                &["c.js"][..],
                Some(
                    r#"2 of the 3 files the manifest names are not in the bundle — "a.js" for one"#,
                ),
            ),
            (
                &[][..],
                Some(
                    r#"none of the 3 files the manifest names are in the bundle — "a.js" for one"#,
                ),
            ),
        ] {
            let output = temp_dir();
            for name in present {
                output.write_file(name, "x");
            }

            let warning = missing_warning(output.path(), &names);

            assert_eq!(warning.as_deref(), counted, "{present:?}");
        }
    }
}
