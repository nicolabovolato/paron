// Which bundler is in use is `manifest`'s business and nothing else's here.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Error, PackageManager};

mod manifest;
mod store;
mod warnings;

pub use manifest::Manifest;

use warnings::{Bundle, unpublishable_warning, unreported_unhashed_name_warning};

/// Where the compressed bundle goes, inside cargo's own output directory for the crate being
/// built. Not beside the bundle: that directory belongs to the bundler, which is entitled to
/// empty it.
const STORE_DIRECTORY: &str = "js-frontend-embed";

/// ```no_run
/// # fn main() -> Result<(), js_frontend_embed::Error> {
/// js_frontend_embed::Build::new("ui").input("openapi.json").run()
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Build {
    directory: PathBuf,
    store: Option<PathBuf>,
    compress: bool,
    package_manager: Option<PackageManager>,
    script: String,
    inputs: Vec<PathBuf>,
    output: PathBuf,
    manifest: Option<Manifest>,
    entry: String,
    environment: Vec<(String, String)>,
    install: bool,
    bundle: bool,
}

impl Build {
    /// `directory` is where the frontend's `package.json` and lockfile are.
    pub fn new(directory: impl AsRef<Path>) -> Self {
        let directory = directory.as_ref().to_owned();

        Self {
            output: directory.join("dist"),
            directory,
            store: None,
            compress: false,
            package_manager: None,
            manifest: None,
            script: "build".to_owned(),
            inputs: Vec::new(),
            entry: crate::ENTRY.to_owned(),
            environment: Vec::new(),
            install: true,
            bundle: true,
        }
    }

    /// Compress the bundle on its way into the binary, so that serving it is a copy rather than
    /// a compression.
    ///
    /// Off by default, which embeds the bundler's output as it is. Turning it on costs a brotli
    /// pass over the bundle per build — at the highest quality in a release build, the lowest in
    /// every other — and gives back a smaller binary, a smaller response, and a request that
    /// spends no CPU on compressing what has not changed since the last one asked.
    ///
    /// [`embedded`](crate::embedded) reads whichever it was given, so nothing else changes with
    /// this.
    #[must_use]
    pub fn compress(mut self, compress: bool) -> Self {
        self.compress = compress;
        self
    }

    /// Where to write the bundle the binary carries — compressed, so that serving it is a copy
    /// rather than a compression.
    ///
    /// Defaults to cargo's own output directory for the crate, which is where a build script
    /// should leave what it makes. Set it when something other than a build script is calling
    /// this, which is the only case where cargo has not said.
    #[must_use]
    pub fn store(mut self, store: impl AsRef<Path>) -> Self {
        self.store = Some(store.as_ref().to_owned());
        self
    }

    /// Override the manager the lockfile implies.
    #[must_use]
    pub fn package_manager(mut self, manager: PackageManager) -> Self {
        self.package_manager = Some(manager);
        self
    }

    /// The `package.json` script that produces the bundle. `build` by convention.
    #[must_use]
    pub fn script(mut self, script: &str) -> Self {
        script.clone_into(&mut self.script);
        self
    }

    /// Where the bundler leaves the bundle. Relative to the frontend's directory, so it reads the
    /// way the bundler's own setting does — `outDir: "dist"` in a Vite config is `.output("dist")`
    /// here. An absolute path is taken as it is.
    #[must_use]
    pub fn output(mut self, output: impl AsRef<Path>) -> Self {
        self.output = self.directory.join(output);
        self
    }

    /// The manifest naming what the bundler emitted — [`Manifest::vite`]. Unset means the
    /// bundler writes none: the bundle still builds, and nothing in it is held forever.
    #[must_use]
    pub fn manifest(mut self, manifest: Manifest) -> Self {
        self.manifest = Some(manifest);
        self
    }

    /// The document the bundler is expected to leave in the output, `index.html` by default. Its
    /// being there is what says a bundle was produced, so a build whose output was wiped runs
    /// again even when nothing changed.
    ///
    /// [`Policy::entry`](crate::Policy::entry) names the same document on the serving side.
    /// Change one and change the other.
    #[must_use]
    pub fn entry(mut self, entry: &str) -> Self {
        // `Path::join` reads a leading slash as "start again from the root".
        entry.trim_start_matches('/').clone_into(&mut self.entry);
        self
    }

    /// A file outside the frontend's directory that it is built from — a generated API schema,
    /// say. Watched and hashed like a source; a missing one is an error.
    #[must_use]
    pub fn input(mut self, path: impl AsRef<Path>) -> Self {
        self.inputs.push(path.as_ref().to_owned());
        self
    }

    /// A variable for the bundler's process, on top of what this build script inherited. Changing
    /// one rebuilds the bundle, since the value can end up inside it.
    ///
    /// The bundler reads it under `key`, and a dev server started with [`crate::dev`] is given the
    /// same. The crate being built reads it under [`crate::PREFIX`] and `key` — so declaring
    /// `OPENAPI_PATH` here is `env!("JS_FRONTEND_EMBED_OPENAPI_PATH")` in Rust, and never takes a
    /// name that crate might want for itself.
    #[must_use]
    pub fn env(mut self, key: &str, value: impl Into<String>) -> Self {
        self.environment.push((key.to_owned(), value.into()));
        self
    }

    /// Whether to install dependencies, on by default. A build with nothing to bundle installs
    /// only when the tree is missing entirely, because `npm ci` answers "nothing to do" by
    /// deleting it and fetching it again.
    ///
    /// Turn it off when something else owns that tree — a CI step, a Nix derivation, a vendored
    /// `node_modules`.
    #[must_use]
    pub fn install(mut self, install: bool) -> Self {
        self.install = install;
        self
    }

    /// Whether to run the script, on by default.
    ///
    /// Turn it off when something else produces the bundle before this runs. A build that finds
    /// no bundle is then an error.
    #[must_use]
    pub fn bundle(mut self, bundle: bool) -> Self {
        self.bundle = bundle;
        self
    }

    /// Installs, bundles, and tells the crate being built where everything went.
    ///
    /// Most runs do no work — the bundle is rebuilt only when the frontend changed — but the
    /// publishing always happens, because the crate reads those values back either way.
    ///
    /// One frontend to a crate, unenforced: the published names are fixed and cargo keeps the
    /// last value given for each, so calling this twice leaves only the second reachable.
    pub fn run(self) -> Result<(), Error> {
        self.publish_paths()?;

        // Before the walk, which would report this as a directory it could not read, and before
        // the lockfile search, which only runs when nobody named a package manager.
        if !self.directory.is_dir() {
            return Err(Error::frontend_missing(&self.directory));
        }

        self.publish_watch_list()?;

        let manager = match self.package_manager {
            Some(manager) => manager,
            None => PackageManager::detect(&self.directory)?,
        };

        let digest = self.digest()?;
        let record = self.record();
        let document = self.entry_document();

        // Two reasons to bundle: the sources moved, or the bundle this configuration asks for is
        // not there. The second covers the settings the digest leaves out — `output`, `entry`.
        let bundling = self.bundle && !(unchanged(&record, &digest) && document.is_file());

        if !self.bundle && !document.is_file() {
            return Err(Error::BundleMissing {
                entry: self.entry.clone(),
                output: self.output.display().to_string(),
            });
        }

        // A dev server started later out of the same directory needs these too, which is why a
        // build that bundles nothing still puts them there when they are missing. It does not
        // install otherwise: `npm ci` answers "nothing to do" by refetching the whole tree.
        if self.install && (bundling || !self.directory.join(DEPENDENCIES).is_dir()) {
            self.run_manager(manager, manager.install())?;
        }

        if bundling {
            self.run_manager(manager, &["run", &self.script])?;

            // Nothing downstream would notice: the embed comes out empty, the binary compiles,
            // and the first sign of it is a blank page.
            if !document.is_file() {
                return Err(Error::NoBundle {
                    script: self.script.clone(),
                    entry: self.entry.clone(),
                    output: self.output.display().to_string(),
                });
            }

            // Taken again rather than reused: writing into its own source tree is ordinary for a
            // bundler, and recording the state that preceded that leaves the next build looking
            // at a change this one caused.
            let settled = self.digest()?;

            write_record(&record, &settled)?;
        }

        // Walked once here and asked several questions below, rather than once per question.
        let bundle = Bundle::read(&self.output)?;

        bundle.warn_about_baggage();

        // `bundling` and not the bundle's own timestamps: a bundler that rewrites a file with the
        // same contents still touches it, and quality 11 over a megabyte is seconds to spend on
        // finding out nothing changed.
        self.publish_store(&bundle, bundling)?;

        // After bundling, either because it just ran or because a previous one left it there.
        self.publish_fingerprints(&bundle)
    }

    /// Compresses the bundle into a store beside the crate's other build output, and publishes
    /// where it went. That store is what the embed carries, so what a deployment serves is
    /// already compressed and a request pays for a copy rather than a compressor.
    ///
    /// A bundler plugin's own `.br` and `.gz` copies are left out of it. They would be a second
    /// copy of assets this already holds, and `warn_about_baggage` has said so.
    fn publish_store(&self, bundle: &Bundle, bundling: bool) -> Result<(), Error> {
        // Nothing to write and nothing to copy: the bundler's own output is what gets embedded,
        // which is what this did before there was a store at all.
        if !self.compress {
            println!(
                "cargo::rustc-env={}={}",
                crate::STORE,
                nameable(&absolute(&self.output)?)?
            );

            return Ok(());
        }

        let store = if let Some(store) = &self.store {
            store.clone()
        } else {
            let out = std::env::var_os("OUT_DIR").ok_or(Error::NoOutputDirectory)?;

            PathBuf::from(out).join(STORE_DIRECTORY)
        };

        // The bundler's manifest names every source path of the frontend, and `embed` keeps it
        // out of the binary with an `exclude` matched against stored names. A compressed name is
        // not the name that pattern was written for, so the manifest is left out here instead —
        // where the name is still the one the pattern knows, and where no pattern can drift.
        let manifest = self
            .manifest
            .as_ref()
            .map(|manifest| self.output.join(manifest.path()));

        let found = bundle.files();
        let alongside: std::collections::HashSet<&Path> =
            found.iter().map(PathBuf::as_path).collect();
        let files: Vec<PathBuf> = found
            .iter()
            .filter(|file| !store::is_a_plugins_copy(file, &alongside))
            .filter(|file| Some(file.as_path()) != manifest.as_deref())
            .cloned()
            .collect();

        let quality = store::quality();

        // Nothing was rebuilt and the store says it holds what this build would write, so what is
        // in there is what would go in there. Anything else — a `cargo clean` that took `OUT_DIR`
        // and left the bundle where it was, a store from a version of this crate that laid one
        // out differently, one filled at another quality — is filled again. The stamp is written
        // last, so an interrupted fill has none and is never mistaken for a finished one.
        if bundling || !store::is_current(&store, quality, &store::Bundle::of(&files)) {
            store::fill(&self.output, &files, &store, quality)?;
        }

        println!(
            "cargo::rustc-env={}={}",
            crate::STORE,
            // Absolute like the other two, because what reads it back is the derive in the crate
            // being built, which resolves a relative folder against its own manifest directory
            // rather than against wherever this was called from.
            nameable(&absolute(&store)?)?
        );

        Ok(())
    }

    /// Absolute, because everything that reads these back is somewhere else: the bundler runs in
    /// the frontend, and the built binary starts wherever a developer was standing.
    #[allow(
        clippy::unnecessary_debug_formatting,
        reason = "`Display` is what would let a path end the instruction it is written into"
    )]
    fn published(&self) -> Result<(PathBuf, PathBuf), Error> {
        let directory = absolute(&self.directory)?;
        let output = absolute(&self.output)?;

        // Refused rather than set aside, unlike a source that cannot be named: a mangled copy of
        // either would point the whole crate at nothing.
        for path in [&directory, &output] {
            if !path.to_str().is_some_and(is_one_line) {
                return Err(Error::UnnameablePath {
                    path: format!("{path:?}"),
                });
            }
        }

        Ok((directory, output))
    }

    /// Called before anything that can return early: the crate being built reads these whether
    /// or not this run had work to do.
    fn publish_paths(&self) -> Result<(), Error> {
        let (directory, output) = self.published()?;

        println!(
            "cargo::rustc-env={}={}",
            crate::DIRECTORY,
            directory.display()
        );
        println!("cargo::rustc-env={}={}", crate::OUTPUT, output.display());
        self.publish_environment();

        Ok(())
    }

    /// What [`Build::env`] declared, twice: once under [`crate::PREFIX`], so the crate being built
    /// can read it with `env!` without this taking a name that crate might want, and once as one
    /// list under the unprefixed names the bundler knows, so a dev server can be given the same
    /// environment the build had.
    ///
    /// A build script says everything in single lines and this list is separated, so a pair that
    /// cannot survive either is warned about and left out rather than published as something
    /// cargo would misread. The same rule the bundler's names go through.
    fn publish_environment(&self) {
        let mut published = Vec::with_capacity(self.environment.len());

        for (key, value) in &self.environment {
            if [key.as_str(), value.as_str()]
                .iter()
                .any(|part| part.contains(['\n', '\r']) || part.contains(crate::SEPARATOR))
            {
                println!(
                    "cargo::warning={key:?} cannot be declared to the frontend: a build script \
                     says everything in single lines, and the list they travel in is separated by \
                     {:?}. The build still gets it; a dev server started from `dev` will not.",
                    crate::SEPARATOR
                );

                continue;
            }

            println!("cargo::rustc-env={}{key}={value}", crate::PREFIX);
            published.push(format!("{key}={value}"));
        }

        println!(
            "cargo::rustc-env={}={}",
            crate::ENVIRONMENT,
            published.join(crate::SEPARATOR)
        );
    }

    /// What cargo should watch, and what it cannot be told about.
    #[allow(
        clippy::unnecessary_debug_formatting,
        reason = "`Display` is what would let a path end the instruction it is written into"
    )]
    fn publish_watch_list(&self) -> Result<(), Error> {
        // The frontend's immediate children rather than the directory itself, which holds
        // `node_modules` and the bundle — both written by this build, so watching the whole thing
        // would rerun it on its own output forever. A file added directly beside them reaches
        // cargo only on the next edit elsewhere, which is the better trade.
        let (watchable, unnameable) = self.watchable()?;

        for path in watchable {
            println!("cargo::rerun-if-changed={path}");
        }

        for path in unnameable {
            // Debug rather than Display: written plainly, the name would end the warning early
            // and hand cargo the rest as an instruction.
            println!(
                "cargo::warning={path:?} cannot be named to cargo, so changes to it will not \
                 rebuild the frontend on their own. Renaming it is the fix."
            );
        }

        Ok(())
    }

    /// What may be held forever: a name is safe to hold only if changing its contents changes
    /// it. Every name in the manifest is taken to carry a content hash, except the names that
    /// [`Manifest::unhashed`] and [`Manifest::unhashed_name`] declare.
    fn publish_fingerprints(&self, bundle: &Bundle) -> Result<(), Error> {
        let names = match &self.manifest {
            // Not an error: a bundler that writes no manifest still builds, and nothing being
            // held forever costs a revalidation per asset rather than a stale one.
            None => Vec::new(),
            Some(manifest) => self.read_manifest(manifest)?,
        };

        bundle.warn_about_missing(&names);

        warnings::warn_about_asset_prefix(&self.entry_document(), &names);

        // No names means no manifest was written, and that has its own warning.
        if let Some(manifest) = &self.manifest
            && !names.is_empty()
        {
            for name in manifest.unreported_unhashed_names(&names) {
                println!("cargo::warning={}", unreported_unhashed_name_warning(name));
            }
        }

        let names = Nameable::of(names);

        for name in &names.unnameable {
            println!("cargo::warning={}", unpublishable_warning(name));
        }

        let fingerprinted = match &self.manifest {
            Some(manifest) => manifest.hashed_names(names.publishable),
            None => Vec::new(),
        };

        println!(
            "cargo::rustc-env={}={}",
            crate::FINGERPRINTED,
            fingerprinted.join(crate::SEPARATOR)
        );

        Ok(())
    }

    #[allow(
        clippy::unnecessary_debug_formatting,
        reason = "`Display` is what would let a path end the instruction it is written into"
    )]
    fn read_manifest(&self, manifest: &Manifest) -> Result<Vec<String>, Error> {
        // The manifest sits inside the bundle, so whoever embeds the bundle embeds it too unless
        // they exclude it. That exclusion is a literal in an attribute and cannot be made to read
        // this, so the next best thing is to say where it is and let a test hold the two together.
        let Some(named) = manifest.path().to_str().filter(|path| is_one_line(path)) else {
            // Refused for the same reason as the two paths above: what reads this back compares
            // it against a name in the bundle, and a mangled copy would quietly match nothing.
            return Err(Error::UnnameablePath {
                path: format!("{:?}", manifest.path()),
            });
        };

        println!("cargo::rustc-env={}={named}", crate::MANIFEST);

        let path = self.output.join(manifest.path());

        match std::fs::read_to_string(&path) {
            Ok(contents) => manifest.read(&contents).ok_or_else(|| Error::Manifest {
                path: path.display().to_string(),
                bundler: manifest.bundler().to_owned(),
            }),
            // The manifest was named, so its absence is the bundler not writing what it was asked
            // for. Not fatal — the bundle is there — but nothing can be cached until it is.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                println!(
                    "cargo::warning={path:?} was not written, so no asset can be cached. Turn \
                     {}'s manifest on — {} — or correct the name given to `Build::manifest`.",
                    manifest.bundler(),
                    manifest.turned_on_by()
                );

                Ok(Vec::new())
            }
            Err(error) => Err(Error::io(&path, error)),
        }
    }

    fn run_manager(&self, manager: PackageManager, arguments: &[&str]) -> Result<(), Error> {
        // Absolute, because the bundler runs with the frontend as its working directory while
        // this path was built against the build script's. A relative one would land somewhere
        // neither of them meant.
        let output =
            std::path::absolute(&self.output).map_err(|error| Error::io(&self.output, error))?;

        let mut command = Command::new(manager.program());
        command
            .args(arguments)
            .current_dir(&self.directory)
            // The same name the crate reads the bundle back from, handed to the thing that writes
            // it. Where the bundle goes is one decision, and a bundler config that reads this has
            // no second copy of it to disagree with.
            .env(crate::OUTPUT, &output);

        for (key, value) in &self.environment {
            command.env(key, value);
        }

        let status = command.status().map_err(|error| match error.kind() {
            // `spawn` reports a working directory that is not there the same way it reports a
            // program that is not installed, and the two send you to opposite ends of the
            // problem. Which one it was is worth one question to the filesystem.
            std::io::ErrorKind::NotFound if self.directory.is_dir() => Error::NotInstalled {
                program: manager.program().to_owned(),
            },
            _ => Error::io(&self.directory, error),
        })?;

        if status.success() {
            return Ok(());
        }

        Err(Error::Failed {
            command: format!("{} {}", manager.program(), arguments.join(" ")),
        })
    }

    fn record(&self) -> PathBuf {
        self.directory.join(RECORD)
    }

    /// Where the bundler is expected to have left the entry document, which is the one thing that
    /// says a bundle was produced at all.
    fn entry_document(&self) -> PathBuf {
        self.output.join(&self.entry)
    }

    /// Everything cargo should watch, split into what it can be told and what it cannot.
    ///
    /// A path that is not UTF-8, or that holds a line ending, cannot go into an instruction as
    /// itself. Written out anyway it would be mangled into the name of a file that does not
    /// exist, and cargo would watch that instead — the same as watching nothing, except that
    /// nobody finds out. The caller says so instead.
    fn watchable(&self) -> Result<(Vec<String>, Vec<PathBuf>), Error> {
        let mut named = Vec::new();
        let mut nameless = Vec::new();

        for path in self
            .watched()?
            .into_iter()
            .chain(self.inputs.iter().cloned())
            .chain(
                crate::package_manager::lockfiles(&self.directory)
                    .into_iter()
                    .map(|(lockfile, _)| lockfile),
            )
            .chain(
                crate::package_manager::package_manager_field(&self.directory)
                    .ok()
                    .flatten()
                    .map(|(package_json, _)| package_json),
            )
        {
            match path.to_str().filter(|text| is_one_line(text)) {
                Some(text) => named.push(text.to_owned()),
                None => nameless.push(path),
            }
        }

        Ok((named, nameless))
    }

    /// The bundler's environment files, which an ignore rule hides for the wrong reason: they
    /// are kept out of a repository because of what is in them, not because a build wrote them,
    /// and a bundler writes what they say straight into the output.
    ///
    /// Only beside the frontend, because that is where a bundler looks for them.
    fn configuration(&self) -> Result<Vec<PathBuf>, Error> {
        let mut found = Vec::new();

        let listing = std::fs::read_dir(&self.directory)
            .map_err(|error| Error::io(&self.directory, error))?;

        for entry in listing {
            let entry = entry.map_err(|error| Error::io(&self.directory, error))?;

            // `.env` and every mode's variant of it: `.env.local`, `.env.production`.
            let named = entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(".env"));

            if named && entry.path().is_file() {
                found.push(entry.path());
            }
        }

        Ok(found)
    }

    /// Gates the bundle, because cargo's own change detection cannot: bundling writes into the
    /// directory it reads from, so a `rerun-if-changed` on the sources would fire on the output.
    ///
    /// The environment goes in as well — a build script is a normal place to hand the bundler a
    /// value that changes, and it reaches the frontend as `import.meta.env`.
    ///
    /// An unspecified hasher is enough: this number is only ever written down and read back here,
    /// so a compiler upgrade changing it costs one rebundle.
    fn digest(&self) -> Result<String, Error> {
        use std::hash::{Hash as _, Hasher as _};

        let mut files = self.sources()?;

        for input in &self.inputs {
            // Told apart, because the two send you to opposite ends of the problem. One says a
            // step that should have run has not; the other says the path is fine and the mistake
            // is what was named.
            if !input.exists() {
                return Err(Error::MissingInput {
                    path: input.display().to_string(),
                });
            }

            if !input.is_file() {
                return Err(Error::InputNotAFile {
                    path: input.display().to_string(),
                });
            }
        }

        files.extend(self.inputs.iter().cloned());
        // A lockfile above the frontend is outside the walk.
        files.extend(
            crate::package_manager::lockfiles(&self.directory)
                .into_iter()
                .map(|(lockfile, _)| lockfile),
        );
        files.extend(
            crate::package_manager::package_manager_field(&self.directory)
                .ok()
                .flatten()
                .map(|(package_json, _)| package_json),
        );
        files.sort();
        files.dedup();

        // Neither a cryptographic hash nor one whose value Rust promises to keep across its own
        // releases, and both are fine for what this is. Nothing here has to survive a chosen
        // collision, because everything that goes in is the project's own; and a compiler that
        // hashes differently costs one rebundle, once, after which the record is written the new
        // way and settles.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();

        self.script.hash(&mut hasher);
        self.environment.hash(&mut hasher);
        self.package_manager.hash(&mut hasher);

        for path in files {
            path.hash(&mut hasher);
            std::fs::read(&path)
                .map_err(|error| Error::io(&path, error))?
                .hash(&mut hasher);
        }

        Ok(format!("{:x}", hasher.finish()))
    }

    fn watched(&self) -> Result<Vec<PathBuf>, Error> {
        let mut paths = Vec::new();

        for entry in self.walk(Some(1)) {
            let entry = entry.map_err(|error| Error::walked(&self.directory, error))?;

            if entry.depth() > 0 && !is_our_record(entry.path()) {
                paths.push(entry.into_path());
            }
        }

        // Deduplicated because a project that does not ignore its env files has them from the
        // walk already, and cargo would otherwise be told about them twice.
        paths.extend(self.configuration()?);
        paths.sort();
        paths.dedup();

        Ok(paths)
    }

    /// A gitignore-respecting walk of the frontend, never yielding what this build itself wrote.
    ///
    /// Linked directories are not descended into: what is on the far side is unbounded, and a
    /// link back at an ancestor is a loop the walk answers with an error. Linked *files* are
    /// read, by `sources`. So edits inside a linked directory rebuild nothing.
    fn walk(&self, depth: Option<usize>) -> ignore::Walk {
        ignore::WalkBuilder::new(&self.directory)
            // Ignore files apply in a tree exported without its history, too.
            .require_git(false)
            // Git's own ignore files and nothing else: `.ignore` is a search convention, and
            // what is noise to search is not what the frontend is built from.
            .ignore(false)
            .hidden(false)
            .max_depth(depth)
            .filter_entry({
                let output = self.output.clone();

                // The bundle is one exact path, because `dist` is an ordinary word for a
                // directory in the sources. Dependencies go by name wherever they appear, since a
                // workspace has a `node_modules` per package. Every real project ignores both;
                // this is for the one that forgot.
                move |entry| {
                    entry.path() != output
                        && entry.file_name() != std::ffi::OsStr::new(DEPENDENCIES)
                }
            })
            .build()
    }

    fn sources(&self) -> Result<Vec<PathBuf>, Error> {
        let mut files = Vec::new();

        for entry in self.walk(None) {
            let entry = entry.map_err(|error| Error::walked(&self.directory, error))?;

            // This build's own record, left out here rather than left to the caller's
            // `.gitignore`. Ignoring answers what not to commit, which is a different question —
            // and a project that never thought to add the line would be hashing a file this build
            // rewrites at the end of every run, so the digest would never come out the same twice
            // and the frontend would rebundle for ever.
            if is_our_record(entry.path()) {
                continue;
            }

            // Asked of the path rather than of the entry, because an entry's type describes a
            // symlink itself rather than what it points at. A linked-in source is an ordinary
            // thing in a workspace, and one skipped here is one whose edits never rebuild
            // anything — while `watched` hands the same link to cargo and has it rerun this
            // build to decide exactly that. Following it here is also what reading it will do.
            if entry.path().is_file() {
                files.push(entry.into_path());
            }
        }

        // The caller sorts and deduplicates, so a project that does not ignore these gets them
        // once whichever way they arrived.
        files.extend(self.configuration()?);

        Ok(files)
    }
}

/// What the last bundle was built from. Beside the frontend rather than in `OUT_DIR`, which
/// `cargo clean` empties while the bundle it describes survives — so every clean would cost a
/// full rebundle. Excluded from both the walk and the digest.
const RECORD: &str = ".js-frontend-embed";
/// Where every manager here leaves what it installed.
const DEPENDENCIES: &str = "node_modules";

/// What a manifest reported, split by whether a build script can name it to cargo at all.
///
/// A separator arrives as two names; a newline ends the line, and cargo reads the next one as
/// another instruction — which Unix lets a file be named. Dropping such a name costs that asset
/// its long cache, which beats failing the build over a comma in a filename.
struct Nameable {
    publishable: Vec<String>,
    unnameable: Vec<String>,
}

impl Nameable {
    fn of(names: Vec<String>) -> Self {
        let (publishable, unnameable) = names
            .into_iter()
            .partition(|name| is_one_line(name) && !name.contains(crate::SEPARATOR));

        Self {
            publishable,
            unnameable,
        }
    }
}

/// Whether a build script can say this inside one of its instructions.
///
/// Cargo reads that output an instruction to a line, so text holding a line ending does not spoil
/// one instruction — it finishes it, and whatever follows is read as the next one. Names reach
/// here from a bundler's manifest and paths from walking a directory, and on Unix a file is free
/// to be called anything; neither is a place a build should be taking instructions from.
fn is_one_line(text: &str) -> bool {
    !text.contains(['\n', '\r'])
}

/// Writes the record in one step rather than two: cargo's lock covers a target directory rather
/// than a package, so an editor's `cargo check` can be inside `run` at the same moment, and
/// truncate-then-fill would let it read half a digest. Writing beside and renaming means the
/// reader sees the old digest or the new one.
///
/// It does not make two concurrent bundles safe. A lock would have to tell a holder that died
/// from one still working, and getting that wrong hangs a build rather than corrupting one.
fn write_record(record: &Path, digest: &str) -> Result<(), Error> {
    let staging = record.with_file_name(format!("{RECORD}.{}", std::process::id()));

    std::fs::write(&staging, digest).map_err(|error| Error::io(&staging, error))?;
    std::fs::rename(&staging, record).map_err(|error| Error::io(record, error))
}

/// Whether a path is something this build wrote for itself rather than part of the frontend.
///
/// By name rather than by exact path, so the staging file [`write_record`] passes through is
/// covered too — it exists for a moment, and a walk running in that moment would otherwise hash
/// it and make the frontend look changed.
fn is_our_record(path: &Path) -> bool {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|name| {
            // The record, or the staging file it is renamed from — the same name with the writing
            // process's id after a dot. Not everything that merely starts the same way: a file
            // someone put in the frontend is theirs however it is spelled, and taking it for one
            // of is_our_record would leave it out of the digest, so that editing it rebuilt nothing.
            name == RECORD
                || name
                    .strip_prefix(RECORD)
                    .is_some_and(|rest| rest.starts_with('.'))
        })
}

/// A path as cargo can be told it, or the refusal the other published paths get: a build script
/// says everything in single lines, and what reads this back has to find the directory.
#[allow(
    clippy::unnecessary_debug_formatting,
    reason = "the same spelling as the other two paths this refuses, and a `Display` path is what \
              would let it end the instruction it is written into"
)]
fn nameable(path: &Path) -> Result<&str, Error> {
    path.to_str()
        .filter(|path| is_one_line(path))
        .ok_or_else(|| Error::UnnameablePath {
            path: format!("{path:?}"),
        })
}

fn absolute(path: &Path) -> Result<PathBuf, Error> {
    std::path::absolute(path).map_err(|error| Error::io(path, error))
}

fn unchanged(record: &Path, digest: &str) -> bool {
    std::fs::read_to_string(record).is_ok_and(|previous| previous == digest)
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::testing::{TempDir, files_under, temp_dir};

    /// A frontend in `ui` with one source file. The store and the files that are not part of the
    /// frontend go beside it, so they do not change the digest.
    struct Frontend {
        root: TempDir,
    }

    impl Frontend {
        fn new() -> Self {
            let root = temp_dir();
            root.write_file("ui/src/main.ts", "export {}");

            Self { root }
        }

        fn path(&self) -> PathBuf {
            self.root.path().join("ui")
        }

        fn store(&self) -> PathBuf {
            self.root.path().join("store")
        }

        fn new_build(&self) -> Build {
            Build::new(self.path()).store(self.store())
        }

        /// A build that bundles with `manager` and does not install.
        fn build_skipping_install(&self, manager: PackageManager) -> Build {
            self.new_build().package_manager(manager).install(false)
        }

        fn write_file(&self, relative: &str, contents: &str) -> PathBuf {
            self.root.write_file(&format!("ui/{relative}"), contents)
        }

        fn write_script(&self, name: &str, command: &str) {
            self.write_file(
                "package.json",
                &serde_json::json!({ "name": "t", "scripts": { name: command } }).to_string(),
            );
        }

        fn read_entry(&self) -> String {
            std::fs::read_to_string(self.path().join("dist").join(crate::ENTRY)).unwrap()
        }

        /// `paths` relative to the frontend.
        fn names_in_frontend(&self, paths: &[impl AsRef<Path>]) -> Vec<String> {
            paths
                .iter()
                .map(|path| {
                    path.as_ref()
                        .strip_prefix(self.path())
                        .unwrap()
                        .display()
                        .to_string()
                })
                .collect()
        }
    }

    const WRITE_ENTRY: &str = "mkdir -p dist && echo built > dist/index.html";

    // A short file does not compress, and the store keeps it as it is.
    const WRITE_LARGE_ENTRY: &str =
        r"mkdir -p dist && head -c 4000 /dev/zero | tr '\0' 'a' > dist/index.html";

    const WRITE_ENTRY_TO_OUTPUT: &str = r#"mkdir -p "$JS_FRONTEND_EMBED_DIST" && echo built > "$JS_FRONTEND_EMBED_DIST/index.html""#;

    /// A frontend with one local dependency, so the install makes `node_modules` without the
    /// network. Its `postinstall` script adds one line to `install-count` beside the frontend for
    /// each install.
    fn npm_frontend_that_counts_installs() -> Frontend {
        let frontend = Frontend::new();
        let install_counter = frontend.root.path().join("install-count");

        frontend.write_file("leaf/package.json", r#"{"name":"leaf","version":"1.0.0"}"#);
        frontend.write_file(
            "package.json",
            &serde_json::json!({
                "name": "t",
                "version": "1.0.0",
                "dependencies": { "leaf": "file:./leaf" },
                "scripts": {
                    "postinstall": format!("echo ran >> '{}'", install_counter.display()),
                    "build": WRITE_ENTRY,
                },
            })
            .to_string(),
        );

        let lockfile_written = Command::new("npm")
            .args([
                "install",
                "--package-lock-only",
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
            ])
            .current_dir(frontend.path())
            .output()
            .expect("the test needs npm on PATH");
        assert!(
            lockfile_written.status.success(),
            "npm could not write a lockfile: {}",
            String::from_utf8_lossy(&lockfile_written.stderr)
        );

        frontend
    }

    fn install_count(frontend: &Frontend) -> usize {
        std::fs::read_to_string(frontend.root.path().join("install-count"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    #[cfg(unix)]
    #[test]
    fn digest_of_an_unreadable_directory_keeps_the_cause() {
        use std::os::unix::fs::PermissionsExt as _;

        let frontend = Frontend::new();
        let locked_directory = frontend.path().join("locked");
        std::fs::create_dir(&locked_directory).unwrap();
        std::fs::set_permissions(&locked_directory, std::fs::Permissions::from_mode(0o000))
            .unwrap();

        let digest = frontend.new_build().digest();

        std::fs::set_permissions(&locked_directory, std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let error = digest.expect_err(
            "this user can read a directory with no permissions, so it is root; run the test as \
             another user",
        );
        assert!(std::error::Error::source(&error).is_some(), "{error}");
    }

    #[test]
    fn digest_excludes_the_record() {
        let frontend = Frontend::new();
        let digest_before = frontend.new_build().digest().unwrap();
        frontend.write_file(RECORD, &digest_before);

        let digest_after = frontend.new_build().digest().unwrap();

        assert_eq!(digest_after, digest_before);
    }

    #[test]
    fn digest_excludes_the_bundle_however_its_path_is_spelled() {
        for output in ["dist", "./dist", "dist/", "./dist/"] {
            let frontend = Frontend::new();
            let digest_before = frontend.new_build().output(output).digest().unwrap();
            frontend.write_file("dist/index.html", "<!doctype html>");
            frontend.write_file("dist/assets/main-a1b2c3.js", "console.log(1)");

            let digest_after = frontend.new_build().output(output).digest().unwrap();

            assert_eq!(digest_after, digest_before, "output spelled {output:?}");
        }
    }

    #[test]
    fn digest_excludes_node_modules_at_any_depth() {
        let frontend = Frontend::new();
        frontend.write_file("packages/ui/src/index.ts", "export {}");
        let digest_before = frontend.new_build().digest().unwrap();
        frontend.write_file(
            "packages/ui/node_modules/left-pad/index.js",
            "module.exports = 1",
        );

        let digest_after = frontend.new_build().digest().unwrap();

        assert_eq!(digest_after, digest_before);
    }

    #[test]
    fn digest_excludes_an_ignored_file() {
        let frontend = Frontend::new();
        frontend.write_file(".gitignore", "*.local\n");
        frontend.write_file("notes.local", "first");
        let digest_before = frontend.new_build().digest().unwrap();
        frontend.write_file("notes.local", "second");

        let digest_after = frontend.new_build().digest().unwrap();

        assert_eq!(digest_after, digest_before);
    }

    #[test]
    fn digest_includes_an_ignored_environment_file() {
        let frontend = Frontend::new();
        frontend.write_file(".gitignore", "*.local\n");
        frontend.write_file(".env.local", "VITE_API=first");
        let digest_before = frontend.new_build().digest().unwrap();
        frontend.write_file(".env.local", "VITE_API=second");

        let digest_after = frontend.new_build().digest().unwrap();

        assert_ne!(digest_after, digest_before);
    }

    // A Python virtualenv is often a directory called `.env`.
    #[test]
    fn digest_with_a_directory_named_like_an_environment_file_returns_ok() {
        let frontend = Frontend::new();
        frontend.write_file(".env/bin/activate", "export VIRTUAL_ENV=.env");

        let digest = frontend.new_build().digest();

        assert_matches!(digest, Ok(_));
    }

    #[test]
    fn digest_changes_when_a_source_is_edited() {
        let frontend = Frontend::new();
        let digest_before = frontend.new_build().digest().unwrap();
        frontend.write_file("src/main.ts", "export const answer = 42");

        let digest_after = frontend.new_build().digest().unwrap();

        assert_ne!(digest_after, digest_before);
    }

    #[test]
    fn digest_changes_when_a_lockfile_above_the_frontend_changes() {
        let frontend = Frontend::new();
        frontend.root.write_file("bun.lock", "one");
        let digest_before = frontend.new_build().digest().unwrap();
        frontend.root.write_file("bun.lock", "two");

        let digest_after = frontend.new_build().digest().unwrap();

        assert_ne!(digest_after, digest_before);
    }

    #[test]
    fn digest_changes_when_the_package_json_with_the_package_manager_field_changes() {
        let frontend = Frontend::new();
        frontend
            .root
            .write_file("package.json", r#"{"packageManager":"bun@1.4.0"}"#);
        let digest_before = frontend.new_build().digest().unwrap();
        frontend
            .root
            .write_file("package.json", r#"{"packageManager":"bun@1.4.1"}"#);

        let digest_after = frontend.new_build().digest().unwrap();

        assert_ne!(digest_after, digest_before);
    }

    #[test]
    fn digest_changes_when_a_setting_for_the_bundler_changes() {
        let frontend = Frontend::new();
        let build = frontend.new_build().env("API_URL", "https://one");
        let digest_before = build.digest().unwrap();

        for (setting, changed_build) in [
            ("env", frontend.new_build().env("API_URL", "https://two")),
            ("script", build.clone().script("build:production")),
        ] {
            assert_ne!(changed_build.digest().unwrap(), digest_before, "{setting}");
        }
    }

    #[test]
    fn digest_changes_when_an_input_is_edited() {
        let frontend = Frontend::new();
        let input = frontend.root.write_file("openapi.json", "{}");
        let digest_before = frontend.new_build().input(&input).digest().unwrap();
        frontend.root.write_file("openapi.json", r#"{"paths":{}}"#);

        let digest_after = frontend.new_build().input(&input).digest().unwrap();

        assert_ne!(digest_after, digest_before);
    }

    #[test]
    fn digest_with_a_missing_input_returns_missing_input() {
        let frontend = Frontend::new();
        let missing_input = frontend.root.path().join("openapi.json");

        let digest = frontend.new_build().input(&missing_input).digest();

        assert_matches!(
            digest,
            Err(Error::MissingInput { path }) if path == missing_input.display().to_string()
        );
    }

    #[test]
    fn digest_with_a_directory_as_input_returns_input_not_a_file() {
        let frontend = Frontend::new();
        let directory = frontend.path().join("src");

        let digest = frontend.new_build().input(&directory).digest();

        assert_matches!(
            digest,
            Err(Error::InputNotAFile { path }) if path == directory.display().to_string()
        );
    }

    // Git reads `.gitignore`. Search tools such as ripgrep also read `.ignore`.
    #[test]
    fn digest_ignores_a_search_tool_ignore_file() {
        let frontend = Frontend::new();
        frontend.write_file("notes.ts", "export const a = 1");
        frontend.write_file(".ignore", "notes.ts\n");
        let digest_before = frontend.new_build().digest().unwrap();
        frontend.write_file("notes.ts", "export const a = 2");

        let digest_after = frontend.new_build().digest().unwrap();

        assert_ne!(digest_after, digest_before);
    }

    #[cfg(unix)]
    #[test]
    fn digest_does_not_follow_a_linked_directory() {
        let frontend = Frontend::new();
        let file_behind_link = frontend
            .root
            .write_file("shared/deep/shared.ts", "export const a = 1");
        std::os::unix::fs::symlink(
            frontend.root.path().join("shared"),
            frontend.path().join("src/linked"),
        )
        .unwrap();
        let digest_before = frontend.new_build().digest().unwrap();
        std::fs::write(&file_behind_link, "export const a = 2").unwrap();

        let digest_after = frontend.new_build().digest().unwrap();

        assert_eq!(digest_after, digest_before);
    }

    #[cfg(unix)]
    #[test]
    fn digest_hashes_a_linked_source() {
        let frontend = Frontend::new();
        let link_target = frontend.root.write_file("shared.ts", "export const a = 1");
        std::os::unix::fs::symlink(&link_target, frontend.path().join("src/linked.ts")).unwrap();
        let digest_before = frontend.new_build().digest().unwrap();
        std::fs::write(&link_target, "export const a = 2").unwrap();

        let digest_after = frontend.new_build().digest().unwrap();

        assert_ne!(digest_after, digest_before);
    }

    #[test]
    fn watched_includes_an_ignored_environment_file() {
        let frontend = Frontend::new();
        frontend.write_file(".gitignore", "*.local\n");
        frontend.write_file(".env.local", "VITE_API=first");
        frontend.write_file("notes.local", "first");

        let watched = frontend.new_build().watched().unwrap();

        assert_eq!(
            frontend.names_in_frontend(&watched),
            [".env.local", ".gitignore", "src"]
        );
    }

    #[test]
    fn watched_names_an_unignored_environment_file_once() {
        let frontend = Frontend::new();
        frontend.write_file(".env", "VITE_API=first");

        let watched = frontend.new_build().watched().unwrap();

        assert_eq!(frontend.names_in_frontend(&watched), [".env", "src"]);
    }

    #[test]
    fn watched_obeys_an_ignore_file_above_the_frontend() {
        let frontend = Frontend::new();
        frontend.write_file("generated/schema.ts", "export {}");
        frontend.root.write_file(".gitignore", "generated/\n");

        let watched = frontend.new_build().watched().unwrap();

        assert_eq!(frontend.names_in_frontend(&watched), ["src"]);
    }

    // The ignore file does not name `node_modules` or `dist`. The walk must leave them out itself.
    #[test]
    fn watched_excludes_what_this_build_wrote() {
        let frontend = Frontend::new();
        frontend.write_file(".gitignore", "*.tsbuildinfo\n");
        frontend.write_file("package.json", "{}");
        frontend.write_file("vite.config.ts", "export default {}");
        frontend.write_file("node_modules/left-pad/index.js", "module.exports = 0");
        frontend.write_file("dist/index.html", "<!doctype html>");
        frontend.write_file("tsconfig.tsbuildinfo", "{}");
        frontend.write_file(RECORD, "digest");

        let watched = frontend.new_build().watched().unwrap();

        assert_eq!(
            frontend.names_in_frontend(&watched),
            [".gitignore", "package.json", "src", "vite.config.ts"]
        );
    }

    #[test]
    fn watchable_names_the_package_json_with_the_package_manager_field() {
        let frontend = Frontend::new();
        let package_json = frontend
            .root
            .write_file("package.json", r#"{"packageManager":"bun@1.4.0"}"#);

        let (named, _) = frontend.new_build().watchable().unwrap();

        assert!(
            named.contains(&package_json.display().to_string()),
            "{named:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn watchable_sets_aside_an_unnameable_path() {
        let frontend = Frontend::new();
        let file_with_newline = frontend.write_file("two\nlines.ts", "export {}");

        let (named, unnameable) = frontend.new_build().watchable().unwrap();

        assert_eq!(frontend.names_in_frontend(&named), ["src"]);
        assert_eq!(unnameable, [file_with_newline]);
    }

    #[test]
    fn entry_with_a_leading_slash_stays_inside_the_output() {
        let build = Build::new("ui").entry("/index.html");

        assert_eq!(build.entry_document(), Path::new("ui/dist/index.html"));
    }

    #[test]
    fn output_relative_is_inside_the_frontend() {
        let build = Build::new("ui").output("built");

        assert_eq!(build.entry_document(), Path::new("ui/built/index.html"));
    }

    #[test]
    fn output_absolute_is_taken_as_it_is() {
        let absolute_output = std::env::temp_dir().join("bundle");

        let build = Build::new("ui").output(&absolute_output);

        assert_eq!(build.entry_document(), absolute_output.join("index.html"));
    }

    #[test]
    fn published_returns_absolute_paths() {
        let build = Build::new("ui").output("built");

        let published = build.published().unwrap();

        let working_directory = std::env::current_dir().unwrap();
        assert_eq!(
            published,
            (
                working_directory.join("ui"),
                working_directory.join("ui/built")
            )
        );
    }

    #[test]
    fn published_with_an_unnameable_path_returns_unnameable_path() {
        let build = Build::new("two\nlines");

        let published = build.published();

        assert_matches!(published, Err(Error::UnnameablePath { .. }));
    }

    // An empty path shows as nothing, and the message would start with a bare colon.
    #[test]
    fn published_with_an_empty_path_names_the_empty_path() {
        let build = Build::new("");

        let published = build.published();

        assert_matches!(
            published,
            Err(Error::Io { path, .. }) if path == "the frontend's directory was given as an empty path"
        );
    }

    #[test]
    fn write_record_is_never_read_half_written() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let directory = temp_dir();
        let record_path = directory.path().join(RECORD);
        let (first_digest, second_digest) = ("a".repeat(64), "b".repeat(64));
        write_record(&record_path, &first_digest).unwrap();
        let is_writing = std::sync::Arc::new(AtomicBool::new(true));
        let reader = std::thread::spawn({
            let record_path = record_path.clone();
            let is_writing = std::sync::Arc::clone(&is_writing);

            move || {
                let mut torn_reads = 0usize;

                while is_writing.load(Ordering::Relaxed) {
                    if let Ok(digest) = std::fs::read_to_string(&record_path)
                        && digest.len() != 64
                    {
                        torn_reads += 1;
                    }
                }

                torn_reads
            }
        });

        for turn in 0..4_000 {
            let digest = if turn % 2 == 0 {
                &second_digest
            } else {
                &first_digest
            };

            write_record(&record_path, digest).unwrap();
        }

        is_writing.store(false, Ordering::Relaxed);
        assert_eq!(reader.join().unwrap(), 0, "torn reads");
    }

    #[test]
    fn write_record_replaces_the_digest_and_leaves_no_staging_file() {
        let directory = temp_dir();
        let record_path = directory.path().join(RECORD);
        write_record(&record_path, "first").unwrap();

        write_record(&record_path, "second").unwrap();

        assert_eq!(files_under(directory.path()), [RECORD]);
        assert_eq!(std::fs::read_to_string(&record_path).unwrap(), "second");
    }

    #[test]
    fn is_our_record_matches_the_record_and_its_staging_file() {
        for (name, is_ours) in [
            (RECORD.to_owned(), true),
            (format!("{RECORD}.4821"), true),
            ("package.json".to_owned(), false),
            ("src".to_owned(), false),
            (format!("{RECORD}ed-notes.md"), false),
        ] {
            assert_eq!(
                is_our_record(&Path::new("ui").join(&name)),
                is_ours,
                "{name}"
            );
        }
    }

    #[test]
    fn read_manifest_reads_a_moved_manifest() {
        let frontend = Frontend::new();
        frontend.write_file(
            "dist/assets.json",
            r#"{"index.html":{"file":"assets/index-D4t.js","isEntry":true}}"#,
        );

        let names = frontend
            .new_build()
            .read_manifest(&Manifest::vite().at("assets.json"))
            .unwrap();

        assert_eq!(names, ["assets/index-D4t.js"]);
    }

    #[test]
    fn read_manifest_without_a_manifest_returns_empty() {
        let frontend = Frontend::new();

        let names = frontend
            .new_build()
            .read_manifest(&Manifest::vite())
            .unwrap();

        assert_eq!(names, Vec::<String>::new());
    }

    #[test]
    fn round_trip_names_through_the_published_line() {
        for names in [
            vec![],
            vec!["assets/index-DkK.js".to_owned()],
            vec![
                "assets/index-DkK.js".to_owned(),
                "assets/logo-B1n.svg".to_owned(),
                "assets/deep/nested-Z18.css".to_owned(),
                "café.png".to_owned(),
                "a name with spaces.svg".to_owned(),
            ],
        ] {
            let line = Nameable::of(names.clone())
                .publishable
                .join(crate::SEPARATOR);

            let names_from_line: Vec<String> = crate::names(&line).map(str::to_owned).collect();

            assert_eq!(names_from_line, names, "through {line:?}");
        }
    }

    #[test]
    fn nameable_of_sets_aside_names_that_cannot_be_published() {
        let nameable = Nameable::of(vec![
            "assets/index-DkK.js".to_owned(),
            "report, final.pdf".to_owned(),
            "sneaky\ncargo::rustc-env=OWNED=yes".to_owned(),
            "windows\r\nstyle.js".to_owned(),
            "assets/logo-B1n.svg".to_owned(),
        ]);

        assert_eq!(
            nameable.publishable,
            ["assets/index-DkK.js", "assets/logo-B1n.svg"]
        );
        assert_eq!(
            nameable.unnameable,
            [
                "report, final.pdf",
                "sneaky\ncargo::rustc-env=OWNED=yes",
                "windows\r\nstyle.js"
            ]
        );
    }

    // The walk gives a different io error for the same directory. The message tells them apart.
    #[test]
    fn run_with_a_missing_frontend_returns_io_error() {
        let frontend = Frontend::new();
        let missing_frontend = frontend.path().join("nowhere");

        let ran = Build::new(&missing_frontend)
            .package_manager(PackageManager::Npm)
            .run();

        assert_eq!(
            ran.unwrap_err().to_string(),
            format!(
                "{}: the frontend is not here, so there is nothing to build",
                missing_frontend.display()
            )
        );
    }

    #[test]
    fn run_without_lockfile_returns_no_lockfile() {
        let frontend = Frontend::new();

        let ran = frontend.new_build().run();

        assert_matches!(ran, Err(Error::NoLockfile { .. }));
    }

    #[test]
    fn run_with_a_named_package_manager_ignores_the_package_manager_field() {
        let frontend = Frontend::new();
        frontend.write_file(
            "package.json",
            &serde_json::json!({
                "name": "t",
                "packageManager": "pnpm@10.0.0",
                "scripts": { "build": WRITE_ENTRY },
            })
            .to_string(),
        );

        let ran = frontend.build_skipping_install(PackageManager::Bun).run();

        assert_matches!(ran, Ok(()));
    }

    #[test]
    fn run_passes_env_to_the_bundler() {
        let frontend = Frontend::new();
        frontend.write_script(
            "build",
            r#"mkdir -p dist && printf '%s' "$WANTED" > dist/index.html"#,
        );

        frontend
            .build_skipping_install(PackageManager::Bun)
            .env("WANTED", "arrived")
            .run()
            .unwrap();

        assert_eq!(frontend.read_entry(), "arrived");
    }

    #[test]
    fn run_looks_for_the_named_entry_document() {
        let frontend = Frontend::new();
        frontend.write_script("build", "mkdir -p dist && echo built > dist/app.html");

        let ran = frontend
            .build_skipping_install(PackageManager::Bun)
            .entry("app.html")
            .run();

        assert_matches!(ran, Ok(()));
    }

    #[test]
    fn run_runs_the_named_script() {
        let frontend = Frontend::new();
        frontend.write_script("bundle", WRITE_ENTRY);

        frontend
            .build_skipping_install(PackageManager::Bun)
            .script("bundle")
            .run()
            .unwrap();

        assert_eq!(frontend.read_entry(), "built\n");
    }

    #[test]
    fn run_without_bundling_does_not_run_the_script() {
        let frontend = Frontend::new();
        frontend.write_script("build", "echo ran > ran");
        frontend.write_file("dist/index.html", "built elsewhere");

        frontend
            .build_skipping_install(PackageManager::Bun)
            .bundle(false)
            .run()
            .unwrap();

        assert!(!frontend.path().join("ran").exists());
    }

    #[test]
    fn run_without_bundling_and_without_a_bundle_returns_bundle_missing() {
        let frontend = Frontend::new();
        frontend.write_script("build", WRITE_ENTRY);

        let ran = frontend
            .build_skipping_install(PackageManager::Bun)
            .bundle(false)
            .run();

        assert_matches!(ran, Err(Error::BundleMissing { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn run_with_nothing_changed_does_not_bundle_again() {
        let frontend = Frontend::new();
        frontend.write_script("build", "mkdir -p dist && echo ran >> dist/index.html");
        frontend
            .build_skipping_install(PackageManager::Bun)
            .run()
            .unwrap();

        frontend
            .build_skipping_install(PackageManager::Bun)
            .run()
            .unwrap();

        assert_eq!(frontend.read_entry(), "ran\n");
    }

    #[cfg(unix)]
    #[test]
    fn run_after_a_source_edit_bundles_again() {
        let frontend = Frontend::new();
        frontend.write_script("build", "mkdir -p dist && echo ran >> dist/index.html");
        frontend
            .build_skipping_install(PackageManager::Bun)
            .run()
            .unwrap();
        frontend.write_file("src/main.ts", "export const answer = 42");

        frontend
            .build_skipping_install(PackageManager::Bun)
            .run()
            .unwrap();

        assert_eq!(frontend.read_entry(), "ran\nran\n");
    }

    #[cfg(unix)]
    #[test]
    fn run_does_not_bundle_again_for_a_source_the_bundler_wrote() {
        let frontend = Frontend::new();
        frontend.write_script(
            "build",
            "mkdir -p dist && echo ran >> dist/index.html && echo generated > src/routes.gen.ts",
        );
        frontend
            .build_skipping_install(PackageManager::Bun)
            .run()
            .unwrap();

        frontend
            .build_skipping_install(PackageManager::Bun)
            .run()
            .unwrap();

        assert_eq!(frontend.read_entry(), "ran\n");
    }

    #[cfg(unix)]
    #[test]
    fn run_bundles_again_when_the_bundle_was_deleted() {
        let frontend = Frontend::new();
        frontend.write_script("build", WRITE_ENTRY_TO_OUTPUT);
        frontend
            .build_skipping_install(PackageManager::Bun)
            .run()
            .unwrap();
        std::fs::remove_dir_all(frontend.path().join("dist")).unwrap();

        frontend
            .build_skipping_install(PackageManager::Bun)
            .run()
            .unwrap();

        assert_eq!(frontend.read_entry(), "built\n");
    }

    #[test]
    fn run_tells_the_bundler_where_the_bundle_goes() {
        let frontend = Frontend::new();
        frontend.write_script("build", WRITE_ENTRY_TO_OUTPUT);

        frontend
            .build_skipping_install(PackageManager::Bun)
            .output("somewhere/else")
            .run()
            .unwrap();

        assert!(frontend.path().join("somewhere/else/index.html").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn run_with_a_failing_bundler_names_the_command() {
        let frontend = Frontend::new();
        frontend.write_script("build", "echo 'tsc: error TS2304' >&2 && exit 1");

        let ran = frontend.build_skipping_install(PackageManager::Bun).run();

        assert_matches!(
            ran,
            Err(Error::Failed { command }) if command == "bun run build"
        );
    }

    // The manifest has the format of webpack-manifest-plugin.
    #[cfg(unix)]
    #[test]
    fn run_with_another_bundlers_manifest_returns_manifest_error() {
        let frontend = Frontend::new();
        frontend.write_script(
            "build",
            r#"mkdir -p dist/.vite && echo built > dist/index.html && printf '{"main.js":"main.abc123.js"}' > dist/.vite/manifest.json"#,
        );

        let ran = frontend
            .build_skipping_install(PackageManager::Bun)
            .manifest(Manifest::vite())
            .run();

        assert_matches!(ran, Err(Error::Manifest { bundler, .. }) if bundler == "Vite");
    }

    #[cfg(unix)]
    #[test]
    fn run_with_the_bundle_elsewhere_returns_no_bundle() {
        let frontend = Frontend::new();
        frontend.write_script("build", "mkdir -p build && echo ran > build/index.html");

        let ran = frontend.build_skipping_install(PackageManager::Bun).run();

        assert_matches!(
            ran,
            Err(Error::NoBundle { output, .. })
                if output == frontend.path().join("dist").display().to_string()
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_with_the_bundle_elsewhere_writes_no_record() {
        let frontend = Frontend::new();
        frontend.write_script("build", "mkdir -p build && echo ran > build/index.html");

        let ran = frontend.build_skipping_install(PackageManager::Bun).run();

        assert_matches!(ran, Err(Error::NoBundle { .. }));
        assert!(!frontend.path().join(RECORD).exists());
    }

    #[cfg(unix)]
    #[test]
    fn run_with_nothing_to_bundle_does_not_install_again() {
        let frontend = npm_frontend_that_counts_installs();
        frontend
            .new_build()
            .package_manager(PackageManager::Npm)
            .run()
            .unwrap();
        let installs_before = install_count(&frontend);

        frontend
            .new_build()
            .package_manager(PackageManager::Npm)
            .run()
            .unwrap();

        assert_eq!(install_count(&frontend), installs_before);
    }

    #[cfg(unix)]
    #[test]
    fn run_after_a_source_edit_installs_again() {
        let frontend = npm_frontend_that_counts_installs();
        frontend
            .new_build()
            .package_manager(PackageManager::Npm)
            .run()
            .unwrap();
        let installs_before = install_count(&frontend);
        frontend.write_file("src/main.ts", "export const answer = 42");

        frontend
            .new_build()
            .package_manager(PackageManager::Npm)
            .run()
            .unwrap();

        assert_eq!(install_count(&frontend), installs_before + 1);
    }

    #[cfg(unix)]
    #[test]
    fn run_without_dependencies_installs_them_again() {
        let frontend = npm_frontend_that_counts_installs();
        frontend
            .new_build()
            .package_manager(PackageManager::Npm)
            .run()
            .unwrap();
        let installs_before = install_count(&frontend);
        std::fs::remove_dir_all(frontend.path().join(DEPENDENCIES)).unwrap();

        frontend
            .new_build()
            .package_manager(PackageManager::Npm)
            .run()
            .unwrap();

        assert_eq!(install_count(&frontend), installs_before + 1);
    }

    #[cfg(unix)]
    #[test]
    fn run_told_not_to_install_does_not_install() {
        let frontend = npm_frontend_that_counts_installs();

        frontend
            .build_skipping_install(PackageManager::Npm)
            .run()
            .unwrap();

        assert_eq!(install_count(&frontend), 0);
    }

    #[cfg(unix)]
    #[test]
    fn run_compressing_fills_the_store() {
        let frontend = Frontend::new();
        frontend.write_script("build", WRITE_LARGE_ENTRY);

        frontend
            .build_skipping_install(PackageManager::Bun)
            .compress(true)
            .run()
            .unwrap();

        assert_eq!(
            files_under(&frontend.store()),
            [".br/index.html", ".compressed"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_compressing_leaves_the_manifest_out_of_the_store() {
        let frontend = Frontend::new();
        frontend.write_script(
            "build",
            &format!(
                r#"{WRITE_LARGE_ENTRY} && mkdir -p dist/.vite && printf '{{"src/main.ts":{{"file":"index.html"}}}}' > dist/.vite/manifest.json"#
            ),
        );

        frontend
            .build_skipping_install(PackageManager::Bun)
            .compress(true)
            .manifest(Manifest::vite())
            .run()
            .unwrap();

        assert_eq!(
            files_under(&frontend.store()),
            [".br/index.html", ".compressed"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_without_compressing_writes_no_store() {
        let frontend = Frontend::new();
        frontend.write_script("build", WRITE_ENTRY);

        frontend
            .build_skipping_install(PackageManager::Bun)
            .run()
            .unwrap();

        assert!(!frontend.store().exists());
    }

    #[cfg(unix)]
    #[test]
    fn run_fills_the_store_again_when_the_bundle_changed_underneath_it() {
        let frontend = Frontend::new();
        frontend.write_script("build", WRITE_LARGE_ENTRY);
        frontend
            .build_skipping_install(PackageManager::Bun)
            .compress(true)
            .run()
            .unwrap();
        frontend.write_file("dist/injected.js", &"b".repeat(4000));

        frontend
            .build_skipping_install(PackageManager::Bun)
            .compress(true)
            .run()
            .unwrap();

        assert_eq!(
            files_under(&frontend.store()),
            [".br/index.html", ".br/injected.js", ".compressed"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_leaves_a_filled_store_alone_when_nothing_was_bundled() {
        let frontend = Frontend::new();
        frontend.write_script("build", WRITE_LARGE_ENTRY);
        frontend
            .build_skipping_install(PackageManager::Bun)
            .compress(true)
            .run()
            .unwrap();
        let stored_entry = frontend.store().join(".br/index.html");
        let modified_before = std::fs::metadata(&stored_entry)
            .unwrap()
            .modified()
            .unwrap();

        frontend
            .build_skipping_install(PackageManager::Bun)
            .compress(true)
            .run()
            .unwrap();

        assert_eq!(
            std::fs::metadata(&stored_entry)
                .unwrap()
                .modified()
                .unwrap(),
            modified_before
        );
    }
}
