use crate::Result;

/// The name and the SID of the current user. `whoami` writes the name in the code page of the
/// console, so only the SID is exact.
pub fn current_user_and_sid() -> Result<(String, String)> {
    let ran = std::process::Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .map_err(|reason| format!("whoami could not be run: {reason}"))?;
    let line = String::from_utf8_lossy(&ran.stdout);

    // "DOMAIN\name","S-1-5-21-…"
    let fields = line
        .trim()
        .strip_prefix('"')
        .and_then(|line| line.strip_suffix('"'))
        .and_then(|line| line.split_once(r#"",""#));

    match fields {
        Some((name, sid)) if ran.status.success() && sid.starts_with("S-1-") => {
            Ok((name.to_owned(), sid.to_owned()))
        }
        _ => Err(format!(
            "could not get the current user from whoami: {}",
            line.trim()
        )
        .into()),
    }
}
