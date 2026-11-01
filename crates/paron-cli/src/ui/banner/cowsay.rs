use rand::seq::IteratorRandom as _;

use crate::ui::palette;

const FACE: &str = "[¬_¬]";

const TEXT: &str = include_str!(env!("PARON_COWSAY_PATH"));

pub fn render() -> String {
    let text = console::style(TEXT.lines().choose(&mut rand::rng()).unwrap_or_default())
        .bold()
        .italic();
    let face = console::style(FACE).fg(palette::accent()).bold();

    format!("{face} {text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_saying_fits_a_narrow_terminal() {
        const NARROW_TERMINAL_COLUMNS: usize = 80;

        for saying in TEXT.lines() {
            let width = FACE.chars().count() + 1 + saying.chars().count();

            assert!(
                width <= NARROW_TERMINAL_COLUMNS,
                "{width} columns: {saying}"
            );
        }
    }
}
