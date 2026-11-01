use std::path::Path;

pub fn write(path: &Path) -> std::io::Result<()> {
    let document = paron_core::infrastructure::http::openapi()
        .to_pretty_json()
        .map_err(std::io::Error::other)?
        + "\n";

    // overwrite the file only if the contents changed
    if std::fs::read_to_string(path).is_ok_and(|contents| contents == document) {
        return Ok(());
    }

    std::fs::write(path, document)
}
