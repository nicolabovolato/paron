// The only part of this crate that knows one bundler from another.

use std::path::{Path, PathBuf};

/// Which bundler wrote the manifest, and where it left it relative to the output directory.
///
/// There is no standard shape, so the bundler is named rather than sniffed at: another one's
/// manifest is JSON that parses and names nothing, which would disable caching silently.
#[derive(Debug, Clone)]
pub struct Manifest {
    path: PathBuf,
    format: Format,
    hashed: bool,
    unhashed_names: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
enum Format {
    Vite,
}

impl Manifest {
    /// Vite with `build.manifest` on, which writes `.vite/manifest.json`.
    ///
    /// ```
    /// # use js_frontend_embed::{Build, Manifest};
    /// Build::new("ui").manifest(Manifest::vite());
    /// ```
    #[must_use]
    pub fn vite() -> Self {
        Self {
            path: PathBuf::from(VITE),
            format: Format::Vite,
            hashed: true,
            unhashed_names: Vec::new(),
        }
    }

    /// The bundler is configured to write names without a content hash, so no name in the
    /// manifest is published as cacheable.
    ///
    /// Without this, every name is taken to carry a hash, which is what bundlers do by default.
    /// Nothing checks it, so a bundler configured without `[hash]` and no `unhashed` here makes
    /// browsers keep stale files.
    #[must_use]
    pub fn unhashed(mut self) -> Self {
        self.hashed = false;
        self
    }

    /// One name in the manifest that has no content hash, so it is not published as cacheable.
    /// The other names are not affected.
    ///
    /// For a file that a plugin emits with a fixed name, such as a service worker. The name is
    /// the path the manifest reports, relative to the output directory.
    ///
    /// ```
    /// # use js_frontend_embed::Manifest;
    /// Manifest::vite().unhashed_name("sw.js");
    /// ```
    #[must_use]
    pub fn unhashed_name(mut self, name: &str) -> Self {
        self.unhashed_names
            .push(name.trim_start_matches('/').to_owned());
        self
    }

    /// The names that may be published as cacheable, out of the names the manifest reported.
    pub(crate) fn hashed_names(&self, names: Vec<String>) -> Vec<String> {
        if !self.hashed {
            return Vec::new();
        }

        names
            .into_iter()
            .filter(|name| !self.unhashed_names.contains(name))
            .collect()
    }

    /// The names given to [`Manifest::unhashed_name`] that the manifest did not report.
    pub(crate) fn unreported_unhashed_names<'a>(&'a self, names: &[String]) -> Vec<&'a str> {
        self.unhashed_names
            .iter()
            .filter(|name| !names.contains(name))
            .map(String::as_str)
            .collect()
    }

    /// Somewhere other than the bundler's default, still relative to the output directory.
    ///
    /// The `#[exclude]` that keeps a manifest out of the binary names wherever it ends up, so
    /// moving it here means moving that too.
    #[must_use]
    pub fn at(mut self, path: impl AsRef<Path>) -> Self {
        // `Path::join` reads a leading slash as a fresh start from the root.
        let path = path.as_ref();
        let relative = path.strip_prefix("/").unwrap_or(path);

        relative.clone_into(&mut self.path);
        self
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn bundler(&self) -> &'static str {
        match self.format {
            Format::Vite => "Vite",
        }
    }

    /// The setting that makes this bundler write one, for the warning that it did not.
    pub(crate) fn turned_on_by(&self) -> &'static str {
        match self.format {
            Format::Vite => "`build.manifest` in vite.config.ts",
        }
    }

    /// The emitted names, or `None` if this is not that bundler's manifest. Naming nothing
    /// counts as not being one.
    pub(crate) fn read(&self, contents: &str) -> Option<Vec<String>> {
        let names = match self.format {
            Format::Vite => vite(contents),
        }?;

        (!names.is_empty()).then_some(names)
    }
}

const VITE: &str = ".vite/manifest.json";

/// `imports` and `dynamicImports` are skipped: they name other chunks by manifest key, which is
/// a source path rather than a file. Each of those chunks has its own entry.
fn vite(manifest: &str) -> Option<Vec<String>> {
    let parsed = serde_json::from_str::<serde_json::Value>(manifest).ok()?;
    let mut names = Vec::new();

    for chunk in parsed.as_object()?.values() {
        names.extend(
            chunk
                .get("file")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        );

        for listed in ["css", "assets"] {
            names.extend(
                chunk
                    .get(listed)
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned),
            );
        }
    }

    names.sort();
    names.dedup();

    Some(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    // What Vite 8 writes. The svg is named twice, and the `imports` keys name no file.
    const VITE_MANIFEST: &str = r#"{
      "index.html": {
        "file": "assets/index-untz88zf.js",
        "name": "index",
        "src": "index.html",
        "isEntry": true,
        "css": ["assets/index-CglnZMF3.css"],
        "assets": ["assets/logo-B1n_snK3.svg"],
        "imports": ["_skeleton-DUGgP8fS.js"],
        "dynamicImports": ["src/routes/login.tsx?tsr-split=component"]
      },
      "src/routes/login.tsx?tsr-split=component": {
        "file": "assets/login-Z18JNx4y.js",
        "name": "login",
        "isDynamicEntry": true
      },
      "_skeleton-DUGgP8fS.js": {
        "file": "assets/skeleton-DUGgP8fS.js",
        "name": "skeleton"
      },
      "src/logo.svg": {
        "file": "assets/logo-B1n_snK3.svg",
        "src": "src/logo.svg"
      }
    }"#;

    #[test]
    fn vite_reports_every_emitted_name_once() {
        assert_eq!(
            vite(VITE_MANIFEST).unwrap(),
            [
                "assets/index-CglnZMF3.css",
                "assets/index-untz88zf.js",
                "assets/login-Z18JNx4y.js",
                "assets/logo-B1n_snK3.svg",
                "assets/skeleton-DUGgP8fS.js",
            ]
        );
    }

    #[test]
    fn vite_of_something_other_than_a_json_object_returns_none() {
        for manifest in ["[]", "not json"] {
            assert_eq!(vite(manifest), None, "{manifest}");
        }
    }

    #[test]
    fn read_another_bundlers_manifest_returns_none() {
        for (bundler, manifest) in [
            (
                "webpack-manifest-plugin",
                r#"{"main.js": "main.abc123.js"}"#,
            ),
            (
                "esbuild metafile",
                r#"{"inputs": {}, "outputs": {"dist/main-ABC.js": {"bytes": 12}}}"#,
            ),
        ] {
            assert_eq!(Manifest::vite().read(manifest), None, "{bundler}");
        }
    }

    #[test]
    fn hashed_names_leaves_out_an_unhashed_name() {
        let manifest = Manifest::vite().unhashed_name("/sw.js");

        let names = manifest.hashed_names(vec![
            "assets/index-DkK_wTPT.js".to_owned(),
            "sw.js".to_owned(),
        ]);

        assert_eq!(names, ["assets/index-DkK_wTPT.js"]);
    }

    #[test]
    fn hashed_names_of_an_unhashed_manifest_returns_empty() {
        let manifest = Manifest::vite().unhashed();

        let names = manifest.hashed_names(vec!["assets/index.js".to_owned()]);

        assert_eq!(names, Vec::<String>::new());
    }

    #[test]
    fn unreported_unhashed_names_returns_the_names_the_manifest_does_not_report() {
        let manifest = Manifest::vite()
            .unhashed_name("sw.js")
            .unhashed_name("assets/sw.js");

        let unreported = manifest.unreported_unhashed_names(&["sw.js".to_owned()]);

        assert_eq!(unreported, ["assets/sw.js"]);
    }

    #[test]
    fn at_absolute_path_stays_inside_the_output_directory() {
        let manifest = Manifest::vite().at("/assets.json");

        assert_eq!(manifest.path, Path::new("assets.json"));
    }
}
