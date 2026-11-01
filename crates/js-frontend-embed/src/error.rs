/// Everything the build script and the dev server can fail at.
#[derive(Debug, thiserror::Error)]
#[allow(
    missing_docs,
    reason = "each variant is documented; its fields are named by its own message"
)]
#[non_exhaustive]
pub enum Error {
    /// Nothing in the frontend, or in a directory above it, says which package manager it is
    /// built with.
    #[error(
        "no lockfile in {directory} or in a directory above it, so there is no way to tell which \
         package manager to use. Commit one, or name the manager with `.package_manager(..)`."
    )]
    NoLockfile { directory: String },

    /// Lockfiles for more than one manager, so which one the project uses is not settled.
    #[error(
        "{directory} has lockfiles for more than one package manager ({lockfiles}), so which one \
         built this frontend is a guess. Delete the ones that are no longer used, or name the \
         manager with `.package_manager(..)`."
    )]
    AmbiguousLockfiles {
        directory: String,
        lockfiles: String,
    },

    /// The `packageManager` field of a `package.json` does not name a supported manager.
    #[error(
        "the packageManager field of {path} is {value}, which does not name a package manager \
         this can run (bun, pnpm or npm). Correct the field, or name the manager with \
         `.package_manager(..)`."
    )]
    UnknownPackageManager { path: String, value: String },

    /// The only lockfile is of a package manager that this does not run.
    #[error(
        "{path} is the lockfile of a package manager this cannot run. Use bun, pnpm or npm, or \
         name the manager with `.package_manager(..)`."
    )]
    UnsupportedLockfile { path: String },

    /// The package manager the lockfile names is not on `PATH`.
    #[error(
        "{program} is not on PATH, and the frontend's lockfile says it is what builds this. \
         Install it, or commit the lockfile of a manager you have."
    )]
    NotInstalled { program: String },

    /// The bundler ran and reported failure, which usually means the frontend itself is broken.
    #[error(
        "`{command}` failed. Whatever it printed on its way out is in the build script output \
         cargo shows above, and is the actual reason."
    )]
    Failed { command: String },

    /// The manifest is there but is not the one the named bundler writes. There is no standard
    /// shape, so another bundler's is JSON that parses and names nothing.
    #[error(
        "{path} does not read as {bundler}'s manifest: nothing in it names a file the bundler \
         emitted. `Build::manifest` has to name the bundler that wrote it."
    )]
    Manifest { path: String, bundler: String },

    /// Cargo did not say where the crate's build output goes, so there is nowhere to put the
    /// compressed bundle the binary carries.
    #[error(
        "OUT_DIR is unset, so this is not running as a build script. `Build::run` belongs in \
         build.rs, where cargo says where its output goes."
    )]
    NoOutputDirectory,

    /// A path the crate being built has to read back cannot be written into a build script's
    /// output as itself.
    #[error(
        "{path} cannot be given to cargo: a build script says everything in single lines, and \
         this holds a line ending or is not UTF-8. Nothing downstream could find the frontend \
         from a mangled copy of it, so move the project somewhere plainer."
    )]
    UnnameablePath { path: String },

    /// The dev server was going to be told to pick its own port.
    #[error(
        "port 0 asks the operating system to choose one, and this has to know the number before \
         anything starts: the bundler is told it, and what forwards to the dev server dials it. \
         Nothing here reads the bundler's output to find out which it got, so choose a port or \
         leave the default."
    )]
    PortNotChosen,

    /// The host the dev server was going to be reached at resolves to no address.
    #[error(
        "{host} does not name anything this machine can reach, and it is where this would go \
         looking for the dev server. Check `Dev::host`: it wants a host on its own, without a \
         scheme."
    )]
    UnknownHost { host: String },

    /// The dev server was going to be told to listen somewhere already taken.
    #[error(
        "{host}:{port} is already in use, and the dev server would be told to listen there. \
         A bundler that finds its port taken moves to the next free one rather than failing, \
         which would leave this forwarding to whatever is already running. Most often it is a \
         dev server from a previous run that was killed rather than shut down; stop it, or move \
         this one with `Dev::port`."
    )]
    PortTaken { host: String, port: u16 },

    /// The bundler succeeded and left nothing where this build was looking for it.
    #[error(
        "`{script}` finished but left no {entry} in {output}. The bundler is writing its bundle \
         somewhere else, and wherever that is — `build.outDir` in a Vite config, `output.path` \
         in webpack's — `Build::output` has to name the same directory."
    )]
    NoBundle {
        script: String,
        entry: String,
        output: String,
    },

    /// The build was told not to bundle, and there is no bundle.
    #[error(
        "there is no {entry} in {output}, and this build does not run the bundler \
         (`Build::bundle(false)`). Produce the bundle first."
    )]
    BundleMissing { entry: String, output: String },

    /// The dev server exited instead of starting to listen.
    #[error(
        "the dev server stopped before it started listening, with {status}. Running the \
         `{script}` script yourself will say why — a bundler that cannot read its own config \
         fails exactly like this."
    )]
    DevServerStopped { script: String, status: String },

    /// The dev server is running but never opened its port.
    #[error(
        "the dev server has been running for {seconds}s without answering on {origin}. The \
         `{script}` script started something, but it is not the dev server, or it is listening \
         somewhere else — which is what a bundler does when its config never reads {variable} \
         and opens whichever port it would have picked on its own.",
        variable = crate::PORT
    )]
    DevServerSilent {
        script: String,
        origin: String,
        seconds: u64,
    },

    /// An input declared with [`Build::input`](crate::Build::input) is not there.
    #[error(
        "{path} is missing, and the frontend is built from it. It is derived, so whatever \
         produces it has to run before this build."
    )]
    MissingInput { path: String },

    /// An input declared with [`Build::input`](crate::Build::input) is there but is not a file.
    #[error(
        "{path} is not a file, and `Build::input` takes one. A directory is not walked here — \
         name the files inside it that the frontend is actually built from."
    )]
    InputNotAFile { path: String },

    /// Somewhere a file was read or written and the filesystem said no.
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
impl Error {
    /// Keeps the cause rather than flattening it.
    #[cfg(feature = "build")]
    pub(crate) fn walked(path: &std::path::Path, error: ignore::Error) -> Self {
        let described = error.to_string();

        Self::io(
            path,
            error
                .into_io_error()
                .unwrap_or_else(|| std::io::Error::other(described)),
        )
    }

    /// The frontend is not where it was said to be. Both the source walk and the lockfile look
    /// can notice this, and should say the same thing.
    #[cfg(any(feature = "build", feature = "serve"))]
    #[allow(dead_code, reason = "`serve` uses it only in a debug build")]
    pub(crate) fn frontend_missing(path: &std::path::Path) -> Self {
        Self::io(
            path,
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the frontend is not here, so there is nothing to build",
            ),
        )
    }

    #[cfg(any(feature = "build", feature = "serve"))]
    #[allow(dead_code, reason = "`serve` uses it only in a debug build")]
    pub(crate) fn io(path: &std::path::Path, source: std::io::Error) -> Self {
        // An empty path renders as nothing, leaving a message that opens with a bare colon.
        let path = if path.as_os_str().is_empty() {
            "the frontend's directory was given as an empty path".to_owned()
        } else {
            path.display().to_string()
        };

        Self::Io { path, source }
    }
}
