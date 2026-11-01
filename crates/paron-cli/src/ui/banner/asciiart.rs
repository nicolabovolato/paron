use console::{Color, Style};

use crate::ui::palette::{self, GRADIENT};

const ASCIIART: [&str; 6] = [
    r"██████╗  █████╗ ██████╗  ██████╗ ███╗   ██╗",
    r"██╔══██╗██╔══██╗██╔══██╗██╔═══██╗████╗  ██║",
    r"██████╔╝███████║██████╔╝██║   ██║██╔██╗ ██║",
    r"██╔═══╝ ██╔══██║██╔══██╗██║   ██║██║╚██╗██║",
    r"██║     ██║  ██║██║  ██║╚██████╔╝██║ ╚████║",
    r"╚═╝     ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═══╝",
];

const SOLID: char = '█';

const GRADIENT_SLANT: usize = 3;
const GRADIENT_SHADOW: f32 = 0.6;

pub fn width() -> usize {
    ASCIIART
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or_default()
}

pub fn height() -> usize {
    ASCIIART.len()
}

pub fn render() -> Vec<String> {
    let paint: fn(usize, &str) -> String = if palette::truecolor() {
        gradient
    } else {
        solid
    };

    ASCIIART
        .iter()
        .enumerate()
        .map(|(row, line)| paint(row, line))
        .collect()
}

fn solid(_row: usize, line: &str) -> String {
    Style::new()
        .fg(palette::accent())
        .apply_to(line)
        .to_string()
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn gradient(row: usize, line: &str) -> String {
    let span = width() - 1 + (height() - 1) * GRADIENT_SLANT;
    let mut painted = String::new();

    for (column, glyph) in line.chars().enumerate() {
        let at = (column + row * GRADIENT_SLANT) as f32 / span as f32;
        let weight = if glyph == SOLID { 1.0 } else { GRADIENT_SHADOW };
        let style = Style::new().fg(mix(at, weight));

        painted.push_str(&style.apply_to(glyph).to_string());
    }

    painted
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn mix(at: f32, weight: f32) -> Color {
    let scaled = at.clamp(0.0, 1.0) * (GRADIENT.len() - 1) as f32;
    let first = (scaled as usize).min(GRADIENT.len() - 2);
    let (from, to, between) = (GRADIENT[first], GRADIENT[first + 1], scaled - first as f32);
    let channel = |from: u8, to: u8| {
        let from = f32::from(from);

        ((from + (f32::from(to) - from) * between) * weight).round() as u8
    };

    Color::TrueColor(
        channel(from.0, to.0),
        channel(from.1, to.1),
        channel(from.2, to.2),
    )
}
