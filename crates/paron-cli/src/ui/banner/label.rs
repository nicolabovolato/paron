use url::{Position, Url};

pub struct Label {
    text: String,
    url: Option<Url>,
}

impl Label {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            url: None,
        }
    }

    pub fn link(url: Url) -> Self {
        Self {
            text: url[Position::BeforeHost..].trim_end_matches('/').to_owned(),
            url: Some(url),
        }
    }

    pub fn width(&self) -> usize {
        console::measure_text_width(&self.text)
    }

    pub fn render(&self) -> String {
        let Some(url) = self.url.as_ref().filter(|_| console::colors_enabled()) else {
            return console::style(&self.text).dim().to_string();
        };

        let styled = console::style(&self.text).dim().underlined();

        format!("\x1b]8;;{url}\x1b\\{styled}\x1b]8;;\x1b\\")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_text_drops_the_scheme_and_the_trailing_slash() {
        for (url, text) in [
            ("https://paron.example/x", "paron.example/x"),
            ("https://paron.example", "paron.example"),
        ] {
            assert_eq!(Label::link(Url::parse(url).unwrap()).text, text, "{url}");
        }
    }
}
