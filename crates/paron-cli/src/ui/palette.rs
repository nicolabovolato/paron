use console::Color;

pub type Rgb = (u8, u8, u8);

pub const GRADIENT: [Rgb; 6] = [
    (231, 181, 81),
    (223, 130, 31),
    (216, 102, 19),
    (208, 79, 24),
    (199, 57, 33),
    (190, 34, 42),
];

const ACCENT: Rgb = GRADIENT[2];
const ACCENT_256: u8 = 166;

pub fn truecolor() -> bool {
    console::true_colors_enabled_stderr()
}

pub fn accent() -> Color {
    if truecolor() {
        Color::TrueColor(ACCENT.0, ACCENT.1, ACCENT.2)
    } else {
        Color::Color256(ACCENT_256)
    }
}
