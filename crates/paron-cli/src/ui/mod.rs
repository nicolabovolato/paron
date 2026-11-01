mod banner;
mod palette;
pub mod wizard;

use std::io::IsTerminal as _;

use dialoguer::theme::SimpleTheme;
use zeroize::Zeroizing;

use crate::Result;
use crate::lifecycle::Stopped;

pub fn print_banner() {
    if std::io::stderr().is_terminal() {
        eprint!("{}", banner::banner());
    }
}

pub fn warn(text: &str) {
    println!(
        "{} {}",
        console::style("WARN:").yellow().bold(),
        console::style(text).yellow()
    );
}

pub fn confirm(question: &str) -> Result<bool> {
    if !is_terminal() {
        return Err(InputRequired.into());
    }

    dialoguer::Confirm::with_theme(&SimpleTheme)
        .with_prompt(question)
        .default(false)
        .interact()
        .map_err(|dialoguer::Error::IO(reason)| interrupted_prompt(reason))
}

pub fn password<T>(question: &str, parse: impl Fn(&str) -> Result<T>) -> Result<T> {
    if !is_terminal() {
        return Err(InputRequired.into());
    }

    let typed = Zeroizing::new(
        dialoguer::Password::with_theme(&SimpleTheme)
            .with_prompt(question)
            .with_confirmation("Type the password again", "the passwords are not the same")
            .validate_with(|typed: &String| parse(typed).map(drop).map_err(|e| e.to_string()))
            .interact()
            .map_err(|dialoguer::Error::IO(reason)| interrupted_prompt(reason))?,
    );

    parse(&typed)
}

pub fn is_interactive(no_prompt: bool) -> bool {
    !no_prompt && is_terminal()
}

fn is_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

fn interrupted_prompt(reason: std::io::Error) -> crate::Error {
    if reason.kind() == std::io::ErrorKind::Interrupted {
        return Stopped::Interrupted.into();
    }

    reason.into()
}

pub fn not_interactive_needs_flag(flag: &'static str) -> impl Fn(crate::Error) -> crate::Error {
    move |reason| match reason.downcast_ref::<InputRequired>() {
        Some(_) => format!("{flag} is required when not running interactively").into(),
        None => reason,
    }
}

#[derive(Debug)]
pub struct InputRequired;

impl std::fmt::Display for InputRequired {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("input required")
    }
}

impl std::error::Error for InputRequired {}
