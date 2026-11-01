use std::path::{Path, PathBuf};

use crate::Result;

pub const FILE: &str = "paron.db";

pub fn mint(dir: &Path) -> Result<PathBuf> {
    let path = dir.join(FILE);

    std::fs::File::create(&path)
        .map_err(|reason| format!("{} could not be written: {reason}", path.display()))?;

    Ok(path)
}

pub fn read(dir: &Path) -> Result<PathBuf> {
    let path = dir.join(FILE);

    if !path.exists() {
        return Err(format!("{FILE} is missing from {}.", dir.display()).into());
    }

    Ok(path)
}
