//! A JavaScript frontend, built into a Rust binary by cargo and nothing else.
//!
//! ```ignore
//! // build.rs — the `build` feature
//! js_frontend_embed::Build::new("ui")
//!     .manifest(js_frontend_embed::Manifest::vite())
//!     .run()
//!     .expect("the frontend builds");
//!
//! // src/main.rs — the `serve` feature
//! #[js_frontend_embed::embed]
//! struct Bundle;
//!
//! // Whether to develop against a bundler or answer from the binary is yours to decide.
//! let frontend = match dev {
//!     Some(dev) => js_frontend_embed::proxying(dev),
//!     None => Bundle::service(),
//! };
//!
//! let app = router.fallback_service(frontend);
//! ```
//!
//! # Features
//!
//! Neither is default: one belongs in `[build-dependencies]` and the other in `[dependencies]`.
//!
//! - `build` — [`Build`], and the [`Manifest`] it has to be told about before anything can be
//!   cached for longer than the page it arrived with.
//! - `serve` — [`embedded`] and the [`Policy`] it answers by, and the [`Embed`] derive that puts
//!   the bundle in the binary in the first place. A debug build also gets `proxying` and the
//!   `Dev` that gives it something to forward to; a release binary carries neither.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// The docs link across both halves, so read them with `--all-features`; anything less reports
// the other half missing.
#![warn(missing_debug_implementations, missing_docs, unreachable_pub)]

#[cfg(feature = "build")]
mod build;
// Only in a debug build: a release binary has no business carrying a bundler supervisor, and
// nothing can reach one. A dependency takes the profile of the build that pulls it in, so this
// is off in release unless `[profile.release] debug-assertions` turns it back on.
#[cfg(all(feature = "serve", debug_assertions))]
mod dev;
mod error;
#[cfg(any(feature = "build", feature = "serve"))]
mod package_manager;
#[cfg(feature = "serve")]
mod serve;
#[cfg(test)]
mod testing;

#[cfg(feature = "build")]
pub use build::{Build, Manifest};
#[cfg(all(feature = "serve", debug_assertions))]
pub use dev::{Dev, DevServer, dev};
pub use error::Error;
#[cfg(any(feature = "build", feature = "serve"))]
pub use package_manager::PackageManager;
#[cfg(feature = "serve")]
pub use rust_embed::Embed;

/// Re-exported so a crate using [`Embed`] needs no `rust-embed` dependency of its own: point the
/// derive's `crate_path` here. It is in this crate's public API, so a major release there is a
/// major release here.
#[cfg(feature = "serve")]
pub use rust_embed;
#[cfg(all(feature = "serve", debug_assertions))]
pub use serve::proxying;
#[cfg(feature = "serve")]
pub use serve::{Policy, embedded, file, served};

/// The frontend the build script published, embedded and ready to serve.
///
/// Goes on a unit struct, which becomes the bundle:
///
/// ```rust,ignore
/// #[js_frontend_embed::embed]
/// struct Bundle;
///
/// let route = Bundle::service();
/// ```
///
/// It expands to the [`Embed`] derive with the three attributes it needs, and to `service()`
/// alongside `directory()`, `fingerprinted()` and `manifest()` — everything the build script
/// published, as functions rather than macros.
///
/// Whether to serve the bundle or forward to a dev server is the caller's, not this attribute's.
/// [`directory()`](Dev) is what a [`dev`] server is started in, and [`proxying`] takes it from
/// there.
///
/// One option, when [`Manifest::at`] moved the bundler's manifest. A pattern rather than a name,
/// because a compressed store keeps it under one more extension than it was written with:
///
/// ```rust,ignore
/// #[js_frontend_embed::embed(manifest = "assets.json*")]
/// struct Bundle;
/// ```
#[cfg(feature = "serve")]
pub use js_frontend_embed_macros::embed;

/// Re-exported so the code [`embed`] expands to needs no `axum` dependency of its own. It is in
/// this crate's public API, so a major release there is a major release here.
#[cfg(feature = "serve")]
pub use axum;

/// The port the dev server is told to listen on, put into its process by [`Dev::spawn`]. The
/// bundler config reads it to decide where to bind.
pub const PORT: &str = "JS_FRONTEND_EMBED_PORT";

/// The default entry document, for [`Build`] and [`Policy`] alike.
pub const ENTRY: &str = "index.html";

/// The frontend's directory, published by [`Build::run`]. Read it with [`embed`], which puts it on `directory()`.
pub const DIRECTORY: &str = "JS_FRONTEND_EMBED_DIR";

/// Where the bundle landed, published by [`Build::run`]. Read it as
/// `$JS_FRONTEND_EMBED_DIST` in an [`Embed`] derive's `folder`, which is what embeds the bundle.
pub const OUTPUT: &str = "JS_FRONTEND_EMBED_DIST";

/// The bundle as it is embedded — compressed, so a response is a copy rather than a compression.
/// Published by [`Build::run`], and read as `$JS_FRONTEND_EMBED_STORE` in an [`Embed`] derive's
/// `folder`, which [`embed`] does for you.
pub const STORE: &str = "JS_FRONTEND_EMBED_STORE";

/// The names the bundler emitted with a content hash, published by [`Build::run`]. Read it with
/// [`embed`], which puts it on `fingerprinted()` and hands it to [`Policy::holding`].
pub const FINGERPRINTED: &str = "JS_FRONTEND_EMBED_FINGERPRINTED";

/// What everything this crate publishes to the compiler is named under, so that [`Build::env`]
/// never takes a name the crate being built might want for itself.
pub const PREFIX: &str = "JS_FRONTEND_EMBED_";

/// What [`Build::env`] declared, published by [`Build::run`] so a dev server can be given the
/// same environment the build had. Read it with [`declared`].
pub const ENVIRONMENT: &str = "JS_FRONTEND_EMBED_ENV";

/// Where the bundler's manifest was looked for, relative to the output directory, published by
/// [`Build::run`] when [`Build::manifest`] named one. Read it with [`embed`], which puts it on `manifest()`.
pub const MANIFEST: &str = "JS_FRONTEND_EMBED_MANIFEST";

/// The directory inside a store that holds the compressed copies, under the names they are served
/// by. A directory rather than an extension, because an extension cannot be told from one an asset
/// already had: a bundle may ship a `dict.br` that is a brotli dictionary the frontend fetches,
/// and reading it as an encoding of `dict` would answer one file with another.
#[cfg(feature = "build")]
pub(crate) const BROTLI: &str = ".br";

/// Written into a store that holds compressed files, so that the serving half can tell one from a
/// bundle embedded as the bundler wrote it — which may hold a `.br` file that is an asset rather
/// than an encoding.
#[cfg(any(feature = "build", feature = "serve"))]
pub(crate) const COMPRESSED: &str = ".compressed";

/// Between names in [`FINGERPRINTED`]: a build script line is one line, so the list is one
/// string.
pub(crate) const SEPARATOR: &str = ",";

/// Public only because [`embed`] expands in the crate being built. Use the attribute.
#[doc(hidden)]
pub fn names(published: &str) -> impl Iterator<Item = &str> {
    published.split(SEPARATOR).filter(|name| !name.is_empty())
}

/// What [`Build::env`] declared, ready for [`Dev::env`]. A value may hold an `=` — only the first
/// one separates — but not a [`SEPARATOR`] or a line ending, which [`Build::run`] warns about and
/// leaves out rather than publishing something cargo would misread.
///
/// Public only because [`embed`] expands in the crate being built. Use the attribute.
#[doc(hidden)]
pub fn declared(published: &str) -> impl Iterator<Item = (&str, &str)> {
    names(published).filter_map(|pair| pair.split_once('='))
}

/// A frontend the build script published, which is what [`embed`] makes of a unit struct.
///
/// Implemented by the attribute, so [`dev`] can take the frontend itself rather than a path and
/// its environment separately.
#[cfg(feature = "serve")]
pub trait Frontend {
    /// Where the frontend's `package.json` is, which is where its dev server runs.
    const DIRECTORY: &'static str;

    /// What [`Build::env`] declared, as [`declared`] reads it.
    const ENVIRONMENT: &'static str;
}
