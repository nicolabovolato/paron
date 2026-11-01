use std::path::{Path, PathBuf};

use crate::Error;

// Bun has two: it changed format at 1.2, and both name the same manager.
const LOCKFILES: &[(&str, PackageManager)] = &[
    ("bun.lock", PackageManager::Bun),
    ("bun.lockb", PackageManager::Bun),
    ("pnpm-lock.yaml", PackageManager::Pnpm),
    ("package-lock.json", PackageManager::Npm),
];

// Without these, a Yarn or Deno project reads as one that forgot to commit a lockfile.
const UNSUPPORTED_LOCKFILES: &[&str] = &["yarn.lock", "deno.lock"];

/// Which package manager builds a frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PackageManager {
    /// `bun.lock`, or the older `bun.lockb`.
    Bun,
    /// `pnpm-lock.yaml`.
    Pnpm,
    /// `package-lock.json`.
    Npm,
}

impl PackageManager {
    /// Reads the manager off the lockfile the project commits.
    ///
    /// The lockfile is the one in `directory`, or in the nearest directory above it that has one:
    /// a frontend that is a member of a workspace has its lockfile at the workspace root.
    ///
    /// Lockfiles for two managers are an error rather than a precedence: the leftover of a
    /// half-finished migration is as likely to be the newer file as the older one, so any order
    /// would install a dependency tree nobody resolved on purpose.
    ///
    /// A `packageManager` field in the `package.json` of `directory`, or of the nearest directory
    /// above it that has the field, is used before any lockfile. A field that names no supported
    /// manager is an error.
    pub fn detect(directory: &Path) -> Result<Self, Error> {
        // Otherwise a frontend that is not where the caller said reads as one that forgot to
        // commit a lockfile.
        if !directory.is_dir() {
            return Err(Error::frontend_missing(directory));
        }

        if let Some((_, manager)) = package_manager_field(directory)? {
            return Ok(manager);
        }

        let found = lockfiles(directory);
        let mut managers: Vec<Self> = Vec::new();

        for (_, manager) in &found {
            if !managers.contains(manager) {
                managers.push(*manager);
            }
        }

        match managers.as_slice() {
            [manager] => Ok(*manager),
            [] => Err(match unsupported_lockfile(directory) {
                Some(lockfile) => Error::UnsupportedLockfile {
                    path: lockfile.display().to_string(),
                },
                None => Error::NoLockfile {
                    directory: directory.display().to_string(),
                },
            }),
            _ => Err(Error::AmbiguousLockfiles {
                directory: found
                    .first()
                    .and_then(|(lockfile, _)| lockfile.parent())
                    .unwrap_or(directory)
                    .display()
                    .to_string(),
                lockfiles: found
                    .iter()
                    .filter_map(|(lockfile, _)| lockfile.file_name()?.to_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            }),
        }
    }
}

/// The lockfiles in `directory`, or in the nearest directory above it that has any. A frontend
/// that is a member of a workspace has none of its own: the one lockfile is at the workspace root.
pub(crate) fn lockfiles(directory: &Path) -> Vec<(PathBuf, PackageManager)> {
    let Ok(directory) = std::path::absolute(directory) else {
        return Vec::new();
    };

    directory
        .ancestors()
        .map(|ancestor| {
            LOCKFILES
                .iter()
                .map(|(lockfile, manager)| (ancestor.join(lockfile), *manager))
                .filter(|(lockfile, _)| lockfile.is_file())
                .collect::<Vec<_>>()
        })
        .find(|found| !found.is_empty())
        .unwrap_or_default()
}

fn unsupported_lockfile(directory: &Path) -> Option<PathBuf> {
    let directory = std::path::absolute(directory).ok()?;

    directory.ancestors().find_map(|ancestor| {
        UNSUPPORTED_LOCKFILES
            .iter()
            .map(|lockfile| ancestor.join(lockfile))
            .find(|lockfile| lockfile.is_file())
    })
}

/// The manager that the `packageManager` field names, and the `package.json` that has the field:
/// the one in `directory`, or the nearest one above it that has the field.
pub(crate) fn package_manager_field(
    directory: &Path,
) -> Result<Option<(PathBuf, PackageManager)>, Error> {
    let Ok(directory) = std::path::absolute(directory) else {
        return Ok(None);
    };

    for ancestor in directory.ancestors() {
        let package_json = ancestor.join("package.json");

        if !package_json.is_file() {
            continue;
        }

        let contents = std::fs::read_to_string(&package_json)
            .map_err(|error| Error::io(&package_json, error))?;
        let parsed = serde_json::from_str::<serde_json::Value>(&contents)
            .map_err(|error| Error::io(&package_json, error.into()))?;

        let Some(field) = parsed.get("packageManager") else {
            continue;
        };

        let name = field
            .as_str()
            .map(|value| value.split_once('@').map_or(value, |(name, _)| name));

        let manager = match name {
            Some("bun") => PackageManager::Bun,
            Some("pnpm") => PackageManager::Pnpm,
            Some("npm") => PackageManager::Npm,
            _ => {
                return Err(Error::UnknownPackageManager {
                    path: package_json.display().to_string(),
                    value: field.to_string(),
                });
            }
        };

        return Ok(Some((package_json, manager)));
    }

    Ok(None)
}

// A release build serves the bundle it was compiled with and never invokes a manager.
#[cfg(any(feature = "build", all(feature = "serve", debug_assertions)))]
impl PackageManager {
    pub(crate) fn program(self) -> &'static str {
        match self {
            Self::Bun => "bun",
            Self::Pnpm => "pnpm",
            Self::Npm => "npm",
        }
    }

    /// Arguments that install exactly what the lockfile says and resolve nothing newer.
    #[cfg(feature = "build")]
    pub(crate) fn install(self) -> &'static [&'static str] {
        match self {
            Self::Bun | Self::Pnpm => &["install", "--frozen-lockfile"],
            Self::Npm => &["ci"],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::testing::{TempDir, temp_dir};

    /// A workspace with a member in `packages/web`, and the path of that member.
    fn workspace_member() -> (TempDir, PathBuf) {
        let workspace = temp_dir();
        let member = workspace.path().join("packages/web");
        std::fs::create_dir_all(&member).unwrap();

        (workspace, member)
    }

    #[test]
    fn detect_each_lockfile_returns_its_manager() {
        for (lockfiles, manager) in [
            (&["bun.lock"][..], PackageManager::Bun),
            (&["bun.lockb"][..], PackageManager::Bun),
            (&["bun.lock", "bun.lockb"][..], PackageManager::Bun),
            (&["pnpm-lock.yaml"][..], PackageManager::Pnpm),
            (&["package-lock.json"][..], PackageManager::Npm),
        ] {
            let directory = temp_dir();
            for lockfile in lockfiles {
                directory.write_file(lockfile, "");
            }

            let detected = PackageManager::detect(directory.path());

            assert_matches!(detected, Ok(found) if found == manager, "{lockfiles:?}");
        }
    }

    #[test]
    fn detect_missing_directory_returns_io_error() {
        let directory = temp_dir();

        let detected = PackageManager::detect(&directory.path().join("missing"));

        assert_matches!(detected, Err(Error::Io { .. }));
    }

    #[test]
    fn detect_without_lockfile_returns_no_lockfile() {
        let directory = temp_dir();

        let detected = PackageManager::detect(directory.path());

        assert_matches!(detected, Err(Error::NoLockfile { .. }));
    }

    #[test]
    fn detect_unsupported_lockfile_returns_unsupported_lockfile() {
        for lockfile in ["yarn.lock", "deno.lock"] {
            let (workspace, member) = workspace_member();
            workspace.write_file(lockfile, "");

            let detected = PackageManager::detect(&member);

            assert_matches!(
                detected,
                Err(Error::UnsupportedLockfile { path }) if path.ends_with(lockfile),
                "{lockfile}"
            );
        }
    }

    #[test]
    fn detect_two_managers_returns_ambiguous_lockfiles() {
        let directory = temp_dir();
        directory.write_file("pnpm-lock.yaml", "");
        directory.write_file("package-lock.json", "");

        let detected = PackageManager::detect(directory.path());

        assert_matches!(
            detected,
            Err(Error::AmbiguousLockfiles { lockfiles, .. })
                if lockfiles == "pnpm-lock.yaml, package-lock.json"
        );
    }

    #[test]
    fn detect_lockfile_above_the_directory_returns_its_manager() {
        let (workspace, member) = workspace_member();
        workspace.write_file("bun.lock", "");

        let detected = PackageManager::detect(&member);

        assert_matches!(detected, Ok(PackageManager::Bun));
    }

    #[test]
    fn detect_lockfiles_at_two_depths_returns_the_nearest() {
        let (workspace, member) = workspace_member();
        workspace.write_file("pnpm-lock.yaml", "");
        workspace.write_file("packages/web/bun.lock", "");

        let detected = PackageManager::detect(&member);

        assert_matches!(detected, Ok(PackageManager::Bun));
    }

    #[test]
    fn detect_package_manager_field_returns_its_manager() {
        for (package_manager_field, manager) in [
            ("bun@1.4.0", PackageManager::Bun),
            ("pnpm@10.0.0+sha512.abc", PackageManager::Pnpm),
            ("npm@11.0.0", PackageManager::Npm),
        ] {
            let directory = temp_dir();
            directory.write_file(
                "package.json",
                format!(r#"{{"name":"t","packageManager":"{package_manager_field}"}}"#),
            );

            let detected = PackageManager::detect(directory.path());

            assert_matches!(detected, Ok(found) if found == manager, "{package_manager_field}");
        }
    }

    #[test]
    fn detect_package_manager_field_above_the_directory_returns_its_manager() {
        let (workspace, member) = workspace_member();
        workspace.write_file("packages/web/package.json", r#"{"name":"web"}"#);
        workspace.write_file(
            "package.json",
            r#"{"name":"workspace","packageManager":"bun@1.4.0"}"#,
        );

        let detected = PackageManager::detect(&member);

        assert_matches!(detected, Ok(PackageManager::Bun));
    }

    #[test]
    fn detect_package_manager_field_and_another_lockfile_returns_the_field() {
        let (workspace, member) = workspace_member();
        workspace.write_file("packages/web/package-lock.json", "");
        workspace.write_file(
            "package.json",
            r#"{"name":"workspace","packageManager":"pnpm@10.0.0"}"#,
        );

        let detected = PackageManager::detect(&member);

        assert_matches!(detected, Ok(PackageManager::Pnpm));
    }

    #[test]
    fn detect_unknown_package_manager_field_returns_unknown_package_manager() {
        for field in [
            r#""vlt@1.0.0""#,
            r#""yarn@4.5.0""#,
            r#""deno@2.0.0""#,
            r#""""#,
            "4",
        ] {
            let directory = temp_dir();
            directory.write_file("bun.lock", "");
            directory.write_file(
                "package.json",
                format!(r#"{{"name":"t","packageManager":{field}}}"#),
            );

            let detected = PackageManager::detect(directory.path());

            assert_matches!(
                detected,
                Err(Error::UnknownPackageManager { value, .. }) if value == field
            );
        }
    }

    #[test]
    fn detect_unparsable_package_json_returns_io_error() {
        let directory = temp_dir();
        directory.write_file("bun.lock", "");
        directory.write_file("package.json", "{");

        let detected = PackageManager::detect(directory.path());

        assert_matches!(detected, Err(Error::Io { .. }));
    }

    #[cfg(feature = "build")]
    #[test]
    fn install_returns_the_frozen_flag_for_each_manager() {
        for (manager, arguments) in [
            (PackageManager::Bun, &["install", "--frozen-lockfile"][..]),
            (PackageManager::Pnpm, &["install", "--frozen-lockfile"][..]),
            (PackageManager::Npm, &["ci"][..]),
        ] {
            assert_eq!(manager.install(), arguments, "{manager:?}");
        }
    }

    // A manager exits with zero for a flag it does not know, so the test expects a refusal.
    #[cfg(feature = "build")]
    #[test]
    fn install_with_an_unlocked_dependency_is_refused() {
        use std::process::Command;

        let package_json_with_dependencies = |dependencies: &str| {
            format!(
                r#"{{"name":"t","version":"1.0.0","dependencies":{{{dependencies}}},"scripts":{{"build":"echo bundled"}}}}"#
            )
        };

        for (manager, lockfile_arguments) in [
            (
                PackageManager::Npm,
                &["install", "--package-lock-only", "--no-audit", "--no-fund"][..],
            ),
            (PackageManager::Pnpm, &["install", "--lockfile-only"][..]),
            (PackageManager::Bun, &["install"][..]),
        ] {
            let program = manager.program();

            let frontend = temp_dir();
            frontend.write_file("leaf/package.json", r#"{"name":"leaf","version":"1.0.0"}"#);
            frontend.write_file(
                "extra/package.json",
                r#"{"name":"extra","version":"1.0.0"}"#,
            );
            frontend.write_file(
                "package.json",
                package_json_with_dependencies(r#""leaf":"file:./leaf""#),
            );
            let lockfile_written = Command::new(program)
                .args(lockfile_arguments)
                .current_dir(frontend.path())
                .output()
                .unwrap_or_else(|error| panic!("the test needs {program} on PATH: {error}"));
            assert!(
                lockfile_written.status.success(),
                "{program} could not write a lockfile: {}",
                String::from_utf8_lossy(&lockfile_written.stderr)
            );

            frontend.write_file(
                "package.json",
                package_json_with_dependencies(r#""leaf":"file:./leaf","extra":"file:./extra""#),
            );
            let install_arguments = manager.install();

            let installed = Command::new(program)
                .args(install_arguments)
                .current_dir(frontend.path())
                .output()
                .unwrap();

            assert!(
                !installed.status.success(),
                "{program} {install_arguments:?} installed a dependency that is not in its lockfile"
            );
        }
    }
}
