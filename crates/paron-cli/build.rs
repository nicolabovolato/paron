use std::path::Path;
use std::process::Command;

#[expect(clippy::expect_used)]
fn main() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("CARGO_MANIFEST_DIR is two directories below the workspace root");

    let target = std::env::var("TARGET").expect("cargo sets TARGET for a build script");

    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("musl") {
        println!("cargo:rustc-link-arg=-Wl,--wrap=getaddrinfo");
    }

    let commit = commit_from_archive(workspace_root).or_else(|| commit_from_git(workspace_root));
    println!(
        "cargo:rustc-env=PARON_VERSION_STR={} ({})",
        env!("CARGO_PKG_VERSION"),
        [commit, Some(target)]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
    );

    let frontend = workspace_root.join("packages/paron-ui");
    let openapi_path = frontend.join("openapi.json");

    println!(
        "cargo:rustc-env=PARON_COWSAY_PATH={}",
        frontend.join("src/assets/cowsay.txt").display()
    );

    paron_openapi::write(&openapi_path).unwrap_or_else(|reason| {
        panic!("{} could not be written: {reason}", openapi_path.display())
    });

    js_frontend_embed::Build::new(frontend)
        .input(&openapi_path)
        .env("OPENAPI_PATH", openapi_path.display().to_string())
        .env("PARON_VERSION", env!("CARGO_PKG_VERSION"))
        .output("./dist")
        .manifest(js_frontend_embed::Manifest::vite())
        .compress(true)
        .run()
        .unwrap_or_else(|reason| {
            panic!("the frontend could not be built: {reason}");
        });
}

const GIT_SHA_ABBREV: usize = 9;

fn commit_from_git(workspace_root: &Path) -> Option<String> {
    let git_dir = workspace_root.join(".git");
    if !git_dir.exists() {
        return None;
    }

    let reflog = git_dir.join("logs/HEAD");
    if reflog.exists() {
        println!("cargo:rerun-if-changed={}", reflog.display());
    }

    let output = Command::new("git")
        .arg("log")
        .arg("-1")
        .arg("--date=short")
        .arg(format!("--abbrev={GIT_SHA_ABBREV}"))
        .arg("--format=%h %cd")
        .current_dir(workspace_root)
        .output()
        .ok()?;

    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn commit_from_archive(workspace_root: &Path) -> Option<String> {
    let stamp = workspace_root.join("git-commit-info");
    if !stamp.exists() {
        return None;
    }

    println!("cargo:rerun-if-changed={}", stamp.display());

    let described = std::fs::read_to_string(&stamp).ok()?;
    let (hash, date) = described.trim().split_once(' ')?;

    (!hash.starts_with('$') && hash.len() >= GIT_SHA_ABBREV)
        .then(|| format!("{} {date}", &hash[..GIT_SHA_ABBREV]))
}
