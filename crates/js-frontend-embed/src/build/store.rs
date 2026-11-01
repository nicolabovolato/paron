//! The bundle as it is embedded, which is not the bundle as it was written.
//!
//! Everything here is compressed once, at build time, instead of once per request for the life of
//! the deployment. A response is bytes copied out of the binary, and the compression that would
//! otherwise happen on a server with a fraction of a core happens on the machine that has a
//! whole one — at a quality no request could afford.
//!
//! Nothing is kept twice. A file whose compressed form is smaller is stored compressed and served
//! that way to anything that says it can take it; the rest is decompressed on the way out, which
//! is the cheap direction and the rare client.

use std::path::{Path, PathBuf};

use crate::Error;

/// What a release build spends. The highest brotli goes, which is minutes of CPU per gigabyte —
/// affordable exactly once.
const BEST: i32 = 11;

/// What every other build spends. The point of compressing in a debug build is that the serving
/// path is the same one a deployment runs, not that the result is small.
const QUICK: i32 = 1;

/// Brotli's window, as a power of two. The largest a decoder is required to support, so the
/// compressor may look back across a whole chunk rather than a sixteenth of one.
const WINDOW: i32 = 22;

/// What went into the store, for the line the build prints.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Stored {
    pub(super) compressed: usize,
    pub(super) verbatim: usize,
    pub(super) before: u64,
    pub(super) after: u64,
}

/// The quality to compress at, from the profile cargo is building.
pub(super) fn quality() -> i32 {
    match std::env::var("PROFILE").as_deref() {
        Ok("release") => BEST,
        _ => QUICK,
    }
}

/// Writes `bundle` into `store`, compressed.
///
/// Built beside the store and moved onto it at the end, so what is there is either a whole store
/// or nothing. Quality 11 over a bundle is seconds, and an interrupt half way through would
/// otherwise leave files that a later build has no way to tell from a finished job — a truncated
/// frontend, embedded, with every build after it agreeing there was nothing to do.
///
/// Names come from the bundle, so this also takes the renamed chunks of the last build with it.
pub(super) fn fill(
    bundle: &Path,
    files: &[PathBuf],
    store: &Path,
    quality: i32,
) -> Result<Stored, Error> {
    let filled = filling(bundle, files, store, quality);

    if filled.is_err() {
        // Half a store, and this build is not going to finish it. Left where it is, the next one
        // would empty it anyway — but it would sit in the output directory until then, and a
        // build that had turned compression off would never come back for it.
        let partial = partial(store);

        match std::fs::remove_dir_all(&partial) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                let shown = partial.to_string_lossy();
                println!("cargo::warning={shown:?} could not be removed: {error}");
            }
        }
    }

    filled
}

fn filling(bundle: &Path, files: &[PathBuf], store: &Path, quality: i32) -> Result<Stored, Error> {
    let described = Bundle::of(files);

    let partial = partial(store);

    empty(&partial)?;

    let mut stored = Stored::default();

    for file in files {
        let name = file
            .strip_prefix(bundle)
            .map_err(|_| Error::io(file, std::io::Error::other("is not inside the bundle")))?;
        let raw = std::fs::read(file).map_err(|error| Error::io(file, error))?;
        let squeezed = squeeze(&raw, quality).map_err(|error| Error::io(file, error))?;

        stored.before += raw.len() as u64;

        // Smaller or it is not worth the encoding: a file that does not compress is one already
        // compressed — a png, a woff2 — and storing brotli around it costs bytes here and a
        // decode in every browser.
        let (name, bytes) = if squeezed.len() < raw.len() {
            stored.compressed += 1;

            (compressed_name(name), squeezed)
        } else {
            stored.verbatim += 1;

            (name.to_path_buf(), raw)
        };

        stored.after += bytes.len() as u64;

        write(&partial.join(name), &bytes)?;
    }

    // Empty, and inside the store: its presence is what tells the serving half that this bundle
    // was compressed on its way in. What it does not hold is when — that would be the build
    // machine's clock, embedded, and two builds of one bundle would stop producing one binary.
    write(&partial.join(crate::COMPRESSED), &[])?;

    swap(&partial, store)?;

    // Last of all, and outside the store: written only once there is a store to describe, so an
    // interrupt anywhere above leaves nothing that claims a finished job.
    if let Some(stamp) = stamp(quality, &described) {
        write(&stamped(store), stamp.as_bytes())?;
    }

    Ok(stored)
}

/// What a filled store says about itself: which version of this crate filled it, what it spent,
/// and what it was filled from.
///
/// Read back before a build decides it has nothing to do. A store from another version may be
/// laid out differently, and one filled quickly is not the one a release should ship — neither is
/// something to discover from a deployment.
///
/// The bundle is described by every name in it with its length and time, because a bundler is not
/// the only thing that writes into an output directory: a step that injects a runtime config, a
/// second build script, someone editing or renaming a built file to try something. None of those
/// touch a source file, so nothing else here would notice, and the binary would go on carrying
/// the bundle from before. A clock that jumps, or a checkout that stamps every file with the time
/// it arrived, costs one compression pass — which is the direction to be wrong in.
fn stamp(quality: i32, bundle: &Bundle) -> Option<String> {
    Some(format!(
        "{} q{quality} {}",
        env!("CARGO_PKG_VERSION"),
        bundle.0?
    ))
}

/// How the bundle looked when the store was filled from it, as one number.
///
/// `None` for a bundle that could not be described — a file whose length or time would not be
/// read. Nothing matches it, so a store is filled again rather than trusted, which is what an
/// unanswerable question deserves.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Bundle(Option<u64>);

impl Bundle {
    pub(super) fn of(files: &[PathBuf]) -> Self {
        use std::hash::{Hash as _, Hasher as _};

        // Order-independent, because a walk is in no particular order: each file is hashed on its
        // own and the results are added together, so the total is the set rather than the list.
        let mut total: u64 = 0;

        for file in files {
            let Ok(found) = std::fs::metadata(file) else {
                return Self(None);
            };

            let Ok(time) = found.modified().and_then(|time| {
                time.duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| std::io::Error::other("before the epoch"))
            }) else {
                return Self(None);
            };

            let mut hasher = std::collections::hash_map::DefaultHasher::new();

            // The name as well as the contents: a rename moves a file without changing its length
            // or its time, and the store is keyed by name.
            file.hash(&mut hasher);
            found.len().hash(&mut hasher);
            time.as_nanos().hash(&mut hasher);

            total = total.wrapping_add(hasher.finish());
        }

        Self(Some(total))
    }
}

/// Whether `store` is one this build would write, so there is nothing to do.
pub(super) fn is_current(store: &Path, quality: i32, bundle: &Bundle) -> bool {
    let Some(stamp) = stamp(quality, bundle) else {
        return false;
    };

    std::fs::read_to_string(stamped(store)).is_ok_and(|found| found == stamp)
}

/// Where the stamp is kept: beside the store, not inside it.
///
/// Inside, it would be embedded — and a stamp holds the time the bundle was written, so two
/// builds of the same bundle would produce different binaries, each carrying the clock of the
/// machine that made it. What goes in the store is the flag, which says only that there is
/// something to say.
fn stamped(store: &Path) -> PathBuf {
    let mut stamped = store.as_os_str().to_owned();
    stamped.push(".stamp");

    PathBuf::from(stamped)
}

/// Where a store is built before it is one. Beside it rather than inside, so that emptying it
/// cannot empty a finished store, and on the same filesystem so the move is a rename.
fn partial(store: &Path) -> PathBuf {
    let mut partial = store.as_os_str().to_owned();
    partial.push(".partial");

    PathBuf::from(partial)
}

/// Replaces `store` with `partial`.
///
/// The window between the two is a store that is not there, which reads as one to fill rather
/// than as one to trust — so an interrupt inside it costs the next build a compression pass and
/// nothing else.
fn swap(partial: &Path, store: &Path) -> Result<(), Error> {
    match std::fs::remove_dir_all(store) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::io(store, error)),
    }

    std::fs::rename(partial, store).map_err(|error| Error::io(partial, error))
}

/// Where a compressed file is stored: under the name it is served by, inside the one directory
/// this crate keeps for itself.
///
/// Not an extension on the name. A bundle may hold a `dict.br` of its own, and a store that said
/// "compressed" with a suffix could not tell that from its own work — so a request for the asset
/// would be answered with an encoding of something else, or refused.
pub(super) fn compressed_name(name: &Path) -> PathBuf {
    Path::new(crate::BROTLI).join(name)
}

/// Whether this is a compressed copy a bundler plugin left beside the file it compresses.
///
/// Those are excluded rather than stored: we compress here, and embedding both would carry the
/// same asset twice. A `.gz` with nothing beside it is a file the frontend fetches and unpacks
/// itself, which is an asset like any other.
pub(super) fn is_a_plugins_copy(name: &Path, alongside: &std::collections::HashSet<&Path>) -> bool {
    if !matches!(
        name.extension().and_then(std::ffi::OsStr::to_str),
        Some("br" | "gz")
    ) {
        return false;
    }

    alongside.contains(name.with_extension("").as_path())
}

fn squeeze(raw: &[u8], quality: i32) -> Result<Vec<u8>, std::io::Error> {
    let mut squeezed = Vec::new();
    let params = brotli::enc::BrotliEncoderParams {
        quality,
        lgwin: WINDOW,
        ..Default::default()
    };

    brotli::BrotliCompress(&mut &raw[..], &mut squeezed, &params)?;

    Ok(squeezed)
}

fn empty(store: &Path) -> Result<(), Error> {
    match std::fs::remove_dir_all(store) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::io(store, error)),
    }

    std::fs::create_dir_all(store).map_err(|error| Error::io(store, error))
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory).map_err(|error| Error::io(directory, error))?;
    }

    std::fs::write(path, bytes).map_err(|error| Error::io(path, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TempDir, files_under, temp_dir};

    const COMPRESSIBLE: &str = "export const answer = 42;\n";

    /// A temporary directory that holds a bundle in `dist`, with the paths of its files.
    fn write_bundle(files: &[(&str, &[u8])]) -> (TempDir, PathBuf, Vec<PathBuf>) {
        let directory = temp_dir();
        let paths = files
            .iter()
            .map(|(name, bytes)| directory.write_file(&format!("dist/{name}"), bytes))
            .collect();
        let bundle = directory.path().join("dist");

        (directory, bundle, paths)
    }

    #[test]
    fn fill_stores_a_file_that_compresses_under_the_brotli_directory() {
        let (directory, bundle, files) =
            write_bundle(&[("assets/app.js", COMPRESSIBLE.repeat(200).as_bytes())]);
        let store = directory.path().join("store");

        let stored = fill(&bundle, &files, &store, QUICK).unwrap();

        assert_eq!((stored.compressed, stored.verbatim), (1, 0));
        assert_eq!(files_under(&store), [".br/assets/app.js", ".compressed"]);
    }

    #[test]
    fn fill_stores_a_file_that_does_not_compress_as_it_is() {
        let incompressible = squeeze(COMPRESSIBLE.repeat(64).as_bytes(), BEST).unwrap();
        let (directory, bundle, files) = write_bundle(&[("assets/photo.png", &incompressible)]);
        let store = directory.path().join("store");

        let stored = fill(&bundle, &files, &store, QUICK).unwrap();

        assert_eq!((stored.compressed, stored.verbatim), (0, 1));
        assert_eq!(
            std::fs::read(store.join("assets/photo.png")).unwrap(),
            incompressible
        );
    }

    #[test]
    fn fill_again_removes_what_the_last_bundle_left() {
        let (directory, old_bundle, old_files) =
            write_bundle(&[("assets/old.js", b"console.log(1)")]);
        let (_new_directory, new_bundle, new_files) =
            write_bundle(&[("assets/new.js", b"console.log(1)")]);
        let store = directory.path().join("store");
        fill(&old_bundle, &old_files, &store, QUICK).unwrap();

        fill(&new_bundle, &new_files, &store, QUICK).unwrap();

        assert_eq!(files_under(&store), [".compressed", "assets/new.js"]);
    }

    #[test]
    fn fill_stores_two_files_with_the_same_stem_apart() {
        let contents = "a".repeat(500);
        let (directory, bundle, files) = write_bundle(&[
            ("index.html", contents.as_bytes()),
            ("index.js", contents.as_bytes()),
        ]);
        let store = directory.path().join("store");

        fill(&bundle, &files, &store, QUICK).unwrap();

        assert_eq!(
            files_under(&store),
            [".br/index.html", ".br/index.js", ".compressed"]
        );
    }

    // The store is embedded. A stamp inside it would put the build time into the binary.
    #[test]
    fn fill_writes_the_stamp_outside_the_store() {
        let (directory, bundle, files) =
            write_bundle(&[("app.js", COMPRESSIBLE.repeat(200).as_bytes())]);
        let store = directory.path().join("store");

        fill(&bundle, &files, &store, QUICK).unwrap();

        assert_eq!(files_under(&store), [".br/app.js", ".compressed"]);
        assert!(stamped(&store).is_file());
    }

    #[test]
    fn fill_writes_an_empty_flag() {
        let (directory, bundle, files) = write_bundle(&[("app.js", b"console.log(1)")]);
        let store = directory.path().join("store");

        fill(&bundle, &files, &store, QUICK).unwrap();

        assert_eq!(
            std::fs::read(store.join(crate::COMPRESSED)).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn is_a_plugins_copy_needs_the_original_beside_it() {
        let alongside: std::collections::HashSet<&Path> =
            [Path::new("app.js"), Path::new("data.gz")]
                .into_iter()
                .collect();

        for (name, is_copy) in [
            ("app.js.br", true),
            ("app.js.gz", true),
            ("dictionary.gz", false),
            ("app.js", false),
        ] {
            assert_eq!(
                is_a_plugins_copy(Path::new(name), &alongside),
                is_copy,
                "{name}"
            );
        }
    }

    #[test]
    fn is_current_after_fill_returns_true() {
        let (directory, bundle, files) =
            write_bundle(&[("assets/app.js", COMPRESSIBLE.repeat(200).as_bytes())]);
        let store = directory.path().join("store");
        fill(&bundle, &files, &store, QUICK).unwrap();

        let store_is_current = is_current(&store, QUICK, &Bundle::of(&files));

        assert!(store_is_current);
    }

    #[test]
    fn is_current_after_a_file_was_renamed_returns_false() {
        let (directory, bundle, files) =
            write_bundle(&[("assets/old.js", COMPRESSIBLE.repeat(200).as_bytes())]);
        let store = directory.path().join("store");
        fill(&bundle, &files, &store, QUICK).unwrap();
        let renamed_file = bundle.join("assets/new.js");
        std::fs::rename(&files[0], &renamed_file).unwrap();

        let store_is_current = is_current(&store, QUICK, &Bundle::of(&[renamed_file]));

        assert!(!store_is_current);
    }

    #[test]
    fn is_current_after_a_file_was_replaced_with_the_same_time_returns_false() {
        let javascript = COMPRESSIBLE.repeat(200);
        let (directory, bundle, files) = write_bundle(&[
            ("assets/app.js", javascript.as_bytes()),
            ("assets/newest.js", javascript.as_bytes()),
        ]);
        let store = directory.path().join("store");
        fill(&bundle, &files, &store, QUICK).unwrap();
        let modified = std::fs::metadata(&files[0]).unwrap().modified().unwrap();
        std::fs::write(&files[0], javascript.repeat(2)).unwrap();
        std::fs::File::open(&files[0])
            .unwrap()
            .set_modified(modified)
            .unwrap();

        let store_is_current = is_current(&store, QUICK, &Bundle::of(&files));

        assert!(!store_is_current);
    }
}
