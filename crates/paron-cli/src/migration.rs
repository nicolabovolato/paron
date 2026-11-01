use std::path::Path;

use indoc::formatdoc;

pub struct Step {
    pub version: u64,
    // Operations must be idempotent and not depend on application code
    pub apply: fn(&Path) -> crate::Result<()>,
}

pub struct Migrations {
    kind: &'static str,
    steps: &'static [Step],
    latest: u64,
}

impl Migrations {
    pub const fn new(kind: &'static str, steps: &'static [Step]) -> Self {
        let mut index = 0;

        while index < kind.len() {
            assert!(
                !kind.as_bytes()[index].is_ascii_whitespace(),
                "kind cannot contain whitespace"
            );

            index += 1;
        }

        assert!(!steps.is_empty(), "specify at least one migration step");

        let mut index = 1;
        while index < steps.len() {
            assert!(
                steps[index - 1].version < steps[index].version,
                "migration steps must be in ascending order"
            );

            index += 1;
        }

        Self {
            kind,
            steps,
            latest: steps[steps.len() - 1].version,
        }
    }

    /// Records the latest version on disk without running migrations
    pub fn stamp(&self, data: &Path) -> crate::Result<()> {
        VersionFile::new(self.kind, self.latest).write(data)
    }

    /// Reads the version stored on disk
    pub fn current(&self, data: &Path) -> crate::Result<Option<u64>> {
        if !data.join(VersionFile::FILENAME).exists() {
            return Ok(None);
        }

        let found = VersionFile::read(data)?;

        if found.kind != self.kind {
            return Err(format!(
                "{}: this directory is set to run Paron {}, not Paron {}",
                data.display(),
                found.kind,
                self.kind
            )
            .into());
        }

        Ok(Some(found.version))
    }

    pub fn apply(&self, data: &Path) -> crate::Result<()> {
        let Some(found) = self.current(data)? else {
            return Err(format!("{}: missing {}", data.display(), VersionFile::FILENAME).into());
        };

        if found > self.latest {
            return Err(format!(
                "{}: this directory needs a newer Paron version",
                data.display()
            )
            .into());
        }

        for step in self.steps.iter().filter(|step| step.version > found) {
            tracing::info!("migrating {} to version {}", data.display(), step.version);

            (step.apply)(data)?;

            VersionFile::new(self.kind, step.version).write(data)?;
        }

        Ok(())
    }
}

struct VersionFile {
    kind: String,
    version: u64,
}

impl VersionFile {
    const FILENAME: &str = "version";

    fn new(kind: &str, version: u64) -> Self {
        Self {
            kind: kind.to_owned(),
            version,
        }
    }

    fn read(dir: &Path) -> crate::Result<Self> {
        let path = dir.join(Self::FILENAME);

        std::fs::read_to_string(&path)
            .map_err(|reason| format!("failed to read {}: {reason}", path.display()))?
            .lines()
            .find(|line| !line.starts_with('#'))
            .unwrap_or_default()
            .parse()
    }

    fn write(&self, dir: &Path) -> crate::Result<()> {
        use std::io::Write as _;

        let Self { kind, version } = self;
        let path = dir.join(Self::FILENAME);

        let contents = formatdoc! {"
            # Written by Paron {paron_version}, do not edit.
            {kind} {version}
            ",
            paron_version = env!("CARGO_PKG_VERSION")
        };

        let tmp_path = dir.join(format!("{}.tmp", Self::FILENAME));

        // Create + rename survives OS failures.
        std::fs::File::create(&tmp_path)
            .and_then(|mut tmp_file| {
                tmp_file.write_all(contents.as_bytes())?;
                tmp_file.sync_all()
            })
            .and_then(|()| std::fs::rename(&tmp_path, &path))
            .map_err(|reason| format!("failed to write {}: {reason}", path.display()))?;

        Ok(())
    }
}

impl std::str::FromStr for VersionFile {
    type Err = crate::Error;

    fn from_str(str: &str) -> Result<Self, Self::Err> {
        let invalid = format!("invalid format: {str:?}");

        let [kind, version] = str.split_whitespace().collect::<Vec<_>>()[..] else {
            return Err(invalid.into());
        };

        Ok(Self::new(kind, version.parse().map_err(|_| invalid)?))
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::testing::TempDataDir;

    const SERVER_KIND: &str = "server";

    const ONE_STEP_MIGRATIONS: Migrations = Migrations::new(
        SERVER_KIND,
        &[Step {
            version: 1,
            apply: |_| Ok(()),
        }],
    );

    const TWO_STEP_MIGRATIONS: Migrations = Migrations::new(
        SERVER_KIND,
        &[
            Step {
                version: 1,
                apply: |_| Ok(()),
            },
            Step {
                version: 2,
                apply: |data_dir| Ok(std::fs::write(data_dir.join("second"), "ran")?),
            },
        ],
    );

    #[test]
    fn apply_without_a_version_file_returns_error() {
        let data_dir = TempDataDir::new("migration-unversioned");

        let error = ONE_STEP_MIGRATIONS.apply(data_dir.path()).unwrap_err();

        assert_eq!(
            error.to_string(),
            format!("{}: missing version", data_dir.path().display())
        );
    }

    #[test]
    fn current_of_empty_directory_returns_none() {
        let data_dir = TempDataDir::new("migration-current-empty");

        let current = ONE_STEP_MIGRATIONS.current(data_dir.path()).unwrap();

        assert_eq!(current, None);
    }

    #[test]
    fn current_after_stamp_returns_latest_version() {
        let data_dir = TempDataDir::new("migration-current");
        ONE_STEP_MIGRATIONS.stamp(data_dir.path()).unwrap();

        let current = ONE_STEP_MIGRATIONS.current(data_dir.path()).unwrap();

        assert_eq!(current, Some(1));
    }

    #[test]
    fn current_of_tunnel_directory_returns_error() {
        let data_dir = TempDataDir::new("migration-current-other");
        VersionFile::new("tunnel", 1)
            .write(data_dir.path())
            .unwrap();

        let error = ONE_STEP_MIGRATIONS.current(data_dir.path()).unwrap_err();

        assert_eq!(
            error.to_string(),
            format!(
                "{}: this directory is set to run Paron tunnel, not Paron server",
                data_dir.path().display()
            )
        );
    }

    #[test]
    fn stamp_records_latest_version_without_running_steps() {
        let data_dir = TempDataDir::new("migration-stamp");

        TWO_STEP_MIGRATIONS.stamp(data_dir.path()).unwrap();

        assert_eq!(VersionFile::read(data_dir.path()).unwrap().version, 2);
        assert!(!data_dir.join("second").exists());
    }

    #[test]
    fn apply_from_version_zero_runs_every_step() {
        let data_dir = TempDataDir::new("migration-walk");
        VersionFile::new(SERVER_KIND, 0)
            .write(data_dir.path())
            .unwrap();

        TWO_STEP_MIGRATIONS.apply(data_dir.path()).unwrap();

        assert_eq!(VersionFile::read(data_dir.path()).unwrap().version, 2);
        assert!(data_dir.join("second").exists());
    }

    #[test]
    fn apply_twice_runs_each_step_once() {
        let data_dir = TempDataDir::new("migration-again");
        VersionFile::new(SERVER_KIND, 0)
            .write(data_dir.path())
            .unwrap();
        TWO_STEP_MIGRATIONS.apply(data_dir.path()).unwrap();
        std::fs::remove_file(data_dir.join("second")).unwrap();

        TWO_STEP_MIGRATIONS.apply(data_dir.path()).unwrap();

        assert!(!data_dir.join("second").exists());
    }

    #[test]
    fn apply_version_newer_than_this_build_returns_error() {
        let data_dir = TempDataDir::new("migration-future");
        VersionFile::new(SERVER_KIND, 9)
            .write(data_dir.path())
            .unwrap();

        let error = ONE_STEP_MIGRATIONS.apply(data_dir.path()).unwrap_err();

        assert_eq!(
            error.to_string(),
            format!(
                "{}: this directory needs a newer Paron version",
                data_dir.path().display()
            )
        );
    }

    #[test]
    fn apply_step_that_fails_keeps_the_version() {
        const FAILING_MIGRATIONS: Migrations = Migrations::new(
            SERVER_KIND,
            &[Step {
                version: 1,
                apply: |_| Err("no".into()),
            }],
        );
        let data_dir = TempDataDir::new("migration-failed");
        VersionFile::new(SERVER_KIND, 0)
            .write(data_dir.path())
            .unwrap();

        let applied = FAILING_MIGRATIONS.apply(data_dir.path());

        assert_matches!(applied, Err(_));
        assert_eq!(VersionFile::read(data_dir.path()).unwrap().version, 0);
    }

    #[test]
    fn apply_malformed_version_file_returns_error() {
        let data_dir = TempDataDir::new("migration-malformed");

        for contents in [
            "not a number\n",
            "server\n",
            "server 1 sealed\nparon 0.1.0\n",
            "# only a comment\n",
        ] {
            std::fs::write(data_dir.join(VersionFile::FILENAME), contents).unwrap();

            let error = ONE_STEP_MIGRATIONS.apply(data_dir.path()).unwrap_err();

            assert!(error.to_string().contains("invalid format"), "{error}");
        }
    }

    #[test]
    fn write_leaves_no_temporary_file() {
        let data_dir = TempDataDir::new("migration-atomic");

        VersionFile::new(SERVER_KIND, 1)
            .write(data_dir.path())
            .unwrap();

        assert_eq!(VersionFile::read(data_dir.path()).unwrap().version, 1);
        assert!(!data_dir.join("version.tmp").exists());
    }

    #[test]
    #[should_panic(expected = "ascending order")]
    fn new_steps_out_of_order_panics() {
        Migrations::new(
            SERVER_KIND,
            &[
                Step {
                    version: 1,
                    apply: |_| Ok(()),
                },
                Step {
                    version: 3,
                    apply: |_| Ok(()),
                },
                Step {
                    version: 2,
                    apply: |_| Ok(()),
                },
            ],
        );
    }

    #[test]
    #[should_panic(expected = "at least one migration step")]
    fn new_without_steps_panics() {
        Migrations::new(SERVER_KIND, &[]);
    }

    #[test]
    fn stamp_writes_the_paron_version_as_a_comment() {
        let data_dir = TempDataDir::new("migration-provenance");

        ONE_STEP_MIGRATIONS.stamp(data_dir.path()).unwrap();

        assert_eq!(
            std::fs::read_to_string(data_dir.join(VersionFile::FILENAME)).unwrap(),
            format!(
                "# Written by Paron {}, do not edit.\nserver 1\n",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn read_skips_comment_lines() {
        let data_dir = TempDataDir::new("migration-comments");
        std::fs::write(
            data_dir.join(VersionFile::FILENAME),
            "# one\n#two\nserver 1\n",
        )
        .unwrap();

        let version_file = VersionFile::read(data_dir.path()).unwrap();

        assert_eq!(version_file.version, 1);
    }
}
