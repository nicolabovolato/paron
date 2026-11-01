mod theme;

use console::style;

use crate::Result;
use crate::lifecycle::Stopped;
use crate::ui::{InputRequired, interrupted_prompt, is_interactive, palette};

pub struct Wizard {
    interactive: bool,
    stopped: bool,
}

impl Wizard {
    pub fn new(no_prompt: bool, title: &str) -> Self {
        let interactive = is_interactive(no_prompt);

        if interactive {
            cliclack::set_theme(theme::Paron);

            let _ = cliclack::intro(style(format!(" {title} ")).bg(palette::accent()).black());
        }

        Self {
            interactive,
            stopped: false,
        }
    }

    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    pub fn input<T>(
        &mut self,
        question: &str,
        default: Option<&str>,
        placeholder: Option<&str>,
        parse: impl Fn(&str) -> Result<T> + Clone + 'static,
    ) -> Result<T> {
        if !self.interactive {
            let Some(default) = default else {
                return Err(InputRequired.into());
            };

            return parse(default);
        }

        let mut input = cliclack::input(question);
        input = input.required(default.is_none());

        if let Some(default) = default.filter(|value| !value.is_empty()) {
            input = input.default_input(default);
        }

        let validate = parse.clone();
        input = input
            .validate(move |typed: &String| validate(typed).map(drop).map_err(|e| e.to_string()));

        if let Some(placeholder) = placeholder {
            input = input.placeholder(placeholder);
        }

        let typed =
            self.record_interrupt(input.interact::<String>().map_err(interrupted_prompt))?;

        parse(&typed)
    }

    pub fn yes_no(&mut self, question: &str, default: bool) -> Result<bool> {
        if !self.interactive {
            return Ok(default);
        }

        let answered = cliclack::confirm(question)
            .initial_value(default)
            .interact()
            .map_err(interrupted_prompt);

        self.record_interrupt(answered)
    }

    pub fn log_input(&mut self, question: &str, answer: &str) {
        if self.interactive {
            let _ = cliclack::log::step(format!("{question}\n{}", style(answer).dim()));
        }
    }

    pub fn install_step(&mut self, text: &str) -> Step {
        if !self.interactive {
            return Step(None);
        }

        let spinner = cliclack::spinner();
        spinner.start(text);

        Step(Some((spinner, text.to_owned())))
    }

    pub fn done(&mut self, text: &str) {
        self.stopped = true;

        if self.interactive {
            let _ = cliclack::outro(style(text).green().bold());
        }
    }

    pub fn cancel(&mut self, text: &str) {
        self.stopped = true;

        if self.interactive {
            let _ = cliclack::outro_cancel(text);
        }
    }

    fn record_interrupt<T>(&mut self, answered: Result<T>) -> Result<T> {
        if let Err(reason) = &answered
            && matches!(reason.downcast_ref::<Stopped>(), Some(Stopped::Interrupted))
        {
            self.stopped = true;
        }

        answered
    }
}

impl Drop for Wizard {
    fn drop(&mut self) {
        if self.interactive && !self.stopped {
            let _ = cliclack::outro_cancel("stopped");
        }
    }
}

pub struct Step(Option<(cliclack::ProgressBar, String)>);

impl Step {
    pub fn finish(mut self) {
        if let Some((spinner, text)) = self.0.take() {
            spinner.stop(text);
        }
    }

    pub fn finish_with(mut self, text: &str) {
        if let Some((spinner, _)) = self.0.take() {
            spinner.stop(text);
        }
    }
}

impl Drop for Step {
    fn drop(&mut self) {
        if let Some((spinner, text)) = self.0.take() {
            spinner.error(text);
        }
    }
}
