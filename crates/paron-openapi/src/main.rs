use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: paron-openapi <path>");
        return ExitCode::from(2);
    };

    if let Err(reason) = paron_openapi::write(&path) {
        eprintln!("{} could not be written: {reason}", path.display());
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
