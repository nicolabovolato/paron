mod asciiart;
mod cowsay;
mod label;

use url::Url;

use label::Label;

const COLUMN_GAP: usize = 3;

pub fn banner() -> String {
    let labels = [
        Label::text(format!("Version {}", env!("CARGO_PKG_VERSION"))),
        Label::text(""),
        Url::parse(env!("CARGO_PKG_HOMEPAGE"))
            .map_or_else(|_| Label::text(env!("CARGO_PKG_HOMEPAGE")), Label::link),
        Url::parse(env!("CARGO_PKG_REPOSITORY"))
            .map_or_else(|_| Label::text(env!("CARGO_PKG_REPOSITORY")), Label::link),
    ];

    let longest = labels.iter().map(Label::width).max().unwrap_or_default();
    let width = usize::from(console::Term::stderr().size().1);

    let banner = if width >= asciiart::width() + COLUMN_GAP + longest {
        horizontal_layout(&labels)
    } else {
        vertical_layout(&labels)
    };

    format!("\n{banner}\n")
}

fn horizontal_layout(labels: &[Label]) -> String {
    let first_row_with_label = asciiart::height().saturating_sub(labels.len()) / 2;
    let mut banner = asciiart::render();

    for (idx, label) in labels.iter().enumerate() {
        if label.width() == 0 {
            continue;
        }

        let row = first_row_with_label + idx;

        while banner.len() <= row {
            banner.push(String::new());
        }

        if row >= asciiart::height() {
            banner[row] = " ".repeat(asciiart::width());
        }

        banner[row].push_str(format!("{}{}", " ".repeat(COLUMN_GAP), label.render()).as_str());
    }

    banner.push(cowsay::render());

    banner.join("\n") + "\n"
}

fn vertical_layout(labels: &[Label]) -> String {
    let mut banner = String::new();

    for line in asciiart::render() {
        banner.push_str(&line);
        banner.push('\n');
    }

    banner.push_str(&cowsay::render());
    banner.push('\n');

    for label in labels {
        if label.width() == 0 {
            banner.push('\n');

            continue;
        }

        let align_right = asciiart::width().saturating_sub(label.width());
        banner.push_str(&" ".repeat(align_right));
        banner.push_str(&label.render());
        banner.push('\n');
    }

    banner
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABEL_TEXTS: [&str; 3] = ["v0.1.0", "paron.example", "github.com/paron"];

    fn text_labels(texts: &[&str]) -> Vec<Label> {
        texts.iter().map(|text| Label::text(*text)).collect()
    }

    #[test]
    fn horizontal_layout_shows_every_label() {
        let labels = text_labels(&LABEL_TEXTS);

        let layout = horizontal_layout(&labels);

        for text in LABEL_TEXTS {
            assert!(layout.contains(text), "{text} is missing");
        }
    }

    #[test]
    fn vertical_layout_shows_every_label() {
        let labels = text_labels(&LABEL_TEXTS);

        let layout = vertical_layout(&labels);

        for text in LABEL_TEXTS {
            assert!(layout.contains(text), "{text} is missing");
        }
    }

    #[test]
    fn horizontal_layout_more_labels_than_art_rows_keeps_the_label_column() {
        let lines = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let labels = text_labels(&lines);

        let layout = horizontal_layout(&labels);

        let banner_lines: Vec<_> = layout
            .lines()
            .map(|line| console::strip_ansi_codes(line).into_owned())
            .collect();
        assert_eq!(banner_lines.len(), lines.len() + 1, "{banner_lines:?}");

        for (row, label) in lines.iter().enumerate() {
            assert!(banner_lines[row].ends_with(label), "{banner_lines:?}");
            assert_eq!(
                banner_lines[row].chars().count() - label.chars().count(),
                asciiart::width() + COLUMN_GAP,
                "{banner_lines:?}"
            );
        }
    }
}
