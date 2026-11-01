/// A command line for the shell of the platform: POSIX `sh`, or PowerShell on Windows
pub fn join(words: impl IntoIterator<Item: AsRef<str>>) -> String {
    let line = words
        .into_iter()
        .map(|word| quoted(word.as_ref()))
        .collect::<Vec<_>>()
        .join(" ");

    // PowerShell reads a quoted first word as a string, and runs it only after `&`.
    #[cfg(windows)]
    if line.starts_with('\'') {
        return format!("& {line}");
    }

    line
}

fn quoted(word: &str) -> String {
    #[cfg(unix)]
    let plain_symbols = "-_./:@%+,";
    #[cfg(windows)]
    let plain_symbols = r"-_./:\";

    let plain =
        |character: char| character.is_ascii_alphanumeric() || plain_symbols.contains(character);

    if !word.is_empty() && word.chars().all(plain) {
        return word.to_owned();
    }

    #[cfg(unix)]
    {
        format!("'{}'", word.replace('\'', r"'\''"))
    }

    #[cfg(windows)]
    {
        let mut quoted = String::from('\'');

        for character in word.chars() {
            // PowerShell also ends a quoted word at the typographic single quotes.
            if matches!(
                character,
                '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}'
            ) {
                quoted.push(character);
            }

            quoted.push(character);
        }

        quoted.push('\'');

        quoted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_plain_words_stay_bare() {
        assert_eq!(
            join(["paron", "server", "run", "--data", "/var/lib/paron"]),
            "paron server run --data /var/lib/paron"
        );
    }

    #[cfg(windows)]
    #[test]
    fn join_quoted_program_runs_with_the_call_operator() {
        assert_eq!(
            join([r"C:\Program Files\Paron\paron.exe", "tunnel", "run"]),
            r"& 'C:\Program Files\Paron\paron.exe' tunnel run"
        );
    }

    #[cfg(windows)]
    #[test]
    fn join_words_survive_powershell() {
        let words = [
            r"C:\Program Files\Paron\paron.exe",
            "it's",
            "‘typographic’",
            "",
            "$HOME",
            "`n",
            "a;b",
            "a|b",
            "a,b",
            "@splat",
            "a#b",
            "(1+1)",
            "ünïcode",
        ];

        let data_dir = crate::testing::TempDataDir::new("powershell-join");
        let script = data_dir.join("print-arguments.ps1");
        // Windows PowerShell reads a script as UTF-8 only with the byte order mark.
        std::fs::write(
            &script,
            format!(
                "\u{feff}[Console]::OutputEncoding = [Text.Encoding]::UTF8\n\
                 & {{ $args | ForEach-Object {{ [Console]::WriteLine($_) }} }} {}\n",
                join(words).trim_start_matches("& ")
            ),
        )
        .unwrap();

        let shell_output = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script)
            .output()
            .expect("PowerShell is part of Windows");

        assert!(shell_output.status.success(), "{shell_output:?}");

        let printed_words: Vec<_> = String::from_utf8(shell_output.stdout)
            .expect("the words went in as text")
            .lines()
            .map(str::to_owned)
            .collect();

        assert_eq!(printed_words, words);
    }

    #[cfg(unix)]
    #[test]
    fn join_words_survive_a_shell() {
        let words = [
            "/opt/pa ron/paron",
            "server",
            "run",
            "--data",
            "/var/lib/pa ron",
            "it's",
            "",
            "~",
            "=ls",
            "a*b",
            "$HOME",
            "`id`",
            "a;b",
            "a|b",
            "a\\b",
            "a#b",
            "ünïcode",
        ];

        let line = join(words);
        let mut shell_count = 0;

        for shell in ["sh", "bash", "zsh"] {
            let Ok(shell_output) = std::process::Command::new(shell)
                .arg("-c")
                .arg(format!("printf '%s\\n' {line}"))
                .output()
            else {
                continue;
            };

            assert!(shell_output.status.success(), "{shell}: {line}");

            let printed_words: Vec<_> = String::from_utf8(shell_output.stdout)
                .expect("the words went in as text")
                .split_terminator('\n')
                .map(str::to_owned)
                .collect();

            assert_eq!(printed_words, words, "{shell}: {line}");

            shell_count += 1;
        }

        assert!(
            shell_count > 0,
            "no shell read the line, so nothing was proven"
        );
    }
}
