use cliclack::{Theme, ThemeState};
use console::{Color, Style};

use crate::ui::palette;

pub struct Paron;

impl Theme for Paron {
    fn bar_color(&self, state: &ThemeState) -> Style {
        match state {
            ThemeState::Active => Style::new().fg(palette::accent()),
            ThemeState::Cancel => Style::new().red(),
            ThemeState::Submit => Style::new().bright().black(),
            ThemeState::Error(_) => Style::new().yellow(),
        }
    }

    fn format_progress_start(&self, template: &str, grouped: bool, last: bool) -> String {
        let space = if grouped { " " } else { "  " };
        let accent = match palette::accent() {
            Color::TrueColor(red, green, blue) => format!("#{red:02x}{green:02x}{blue:02x}"),
            Color::Color256(index) => index.to_string(),
            _ => String::new(),
        };

        self.format_progress_with_state(
            &format!("{{spinner:.{accent}}}{space}{template}"),
            grouped,
            last,
            &ThemeState::Active,
        )
    }
}
