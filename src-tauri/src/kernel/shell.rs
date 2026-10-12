//! Display form of a launch command (design §3.4).
//!
//! The launcher passes argv straight to the OS without a shell, so none of this affects what
//! the browser receives. It only renders the copyable command line in the editor's summary
//! panel, quoted so that pasting it into the host's shell yields the same argv.
//!
//! Target shells: `sh`, bash and zsh on Linux / macOS; `cmd.exe` on Windows. PowerShell is not
//! a target: it needs `& ` before a quoted program path and expands `$` inside double quotes.
//! `cmd.exe` also expands `%name%` even inside quotes, which cannot be escaped there; a value
//! such as a percent-encoded URL is shown as it is.

/// Whose quoting rules the displayed command line follows. Passed explicitly rather than read
/// from `cfg`, so both styles are testable on any host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPlatform {
    /// POSIX `sh` quoting, which bash and zsh (the macOS default) read the same way.
    Unix,
    /// `CommandLineToArgvW` quoting (how Chromium splits its command line on Windows),
    /// escaped for `cmd.exe`.
    Windows,
}

impl HostPlatform {
    /// The platform the launcher is running on.
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// `program arg1 arg2 ...`, each word quoted for `host`.
pub fn command_line(program: &str, args: &[String], host: HostPlatform) -> String {
    let mut line = quote(program, host);
    for arg in args {
        line.push(' ');
        line.push_str(&quote_arg(arg, host));
    }
    match host {
        HostPlatform::Unix => line,
        HostPlatform::Windows => escape_for_cmd(&line),
    }
}

/// Quotes one argument. A `--name=value` switch keeps its name bare and quotes only the value
/// (`--user-data-dir='/tmp/my dir'`), as the upstream README writes them.
fn quote_arg(arg: &str, host: HostPlatform) -> String {
    match arg.split_once('=') {
        Some((name, value)) if is_switch_name(name) => {
            if value.is_empty() {
                arg.to_owned()
            } else {
                format!("{name}={}", quote(value, host))
            }
        }
        _ => quote(arg, host),
    }
}

/// `-x` / `--name-with_dashes`: nothing in it needs quoting under either style.
fn is_switch_name(name: &str) -> bool {
    name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn quote(word: &str, host: HostPlatform) -> String {
    match host {
        HostPlatform::Unix => quote_posix(word),
        HostPlatform::Windows => quote_windows(word),
    }
}

/// Bare when every byte is in `[A-Za-z0-9_@%+=:,./-]` and the word does not start with `=`
/// (zsh expands `=name` to the path of the command `name`), otherwise single-quoted with each
/// `'` written as `'\''`. Non-ASCII text is quoted and kept as it is.
fn quote_posix(word: &str) -> String {
    let safe = |b: u8| b.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&b);
    if !word.is_empty() && !word.starts_with('=') && word.bytes().all(safe) {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// Double-quoted when the word is empty or contains whitespace, `"`, or a `cmd.exe`
/// metacharacter (`& | < > ^`, which cmd leaves alone inside quotes; see [`escape_for_cmd`]).
/// Inside the quotes, backslashes are literal except before a `"` or the closing quote, where
/// they are doubled (`CommandLineToArgvW` rules).
fn quote_windows(word: &str) -> String {
    let needs_quotes = word.is_empty()
        || word
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '&' | '|' | '<' | '>' | '^'));
    if !needs_quotes {
        return word.to_owned();
    }
    let mut out = String::with_capacity(word.len() + 2);
    out.push('"');
    let mut backslashes = 0;
    for c in word.chars() {
        if c == '\\' {
            backslashes += 1;
            continue;
        }
        let escaped = if c == '"' {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        out.extend(std::iter::repeat_n('\\', escaped));
        out.push(c);
        backslashes = 0;
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// `cmd.exe` flips its own quote state at every `"`, including the `\"` that
/// `CommandLineToArgvW` reads as a literal quote. After an odd number of embedded quotes, cmd's
/// state is the inverse of ours, and a metacharacter later in the line (in the same word or a
/// later one) would sit outside cmd's quotes. Such metacharacters get a `^`, which cmd removes.
/// Lines without embedded quotes are unchanged, since bare words never contain metacharacters.
fn escape_for_cmd(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            '&' | '|' | '<' | '>' | '^' if !quoted => out.push('^'),
            _ => {}
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Splits a command line the way `CommandLineToArgvW` splits the arguments after the program
    /// name (its undocumented corners as reproduced by Wine's `CommandLineToArgvW`):
    /// - space and tab separate words outside quotes;
    /// - `2n` backslashes before `"` yield `n` and the `"` toggles quoting; `2n+1` yield `n` and
    ///   a literal `"`; backslashes not before `"` are literal;
    /// - inside quotes, `""` yields a literal `"` and ends the quoted part.
    pub(crate) fn windows_split(line: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut word = String::new();
        let mut in_word = false;
        let mut quoted = false;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    let mut n = 1;
                    while chars.next_if_eq(&'\\').is_some() {
                        n += 1;
                    }
                    if chars.peek() == Some(&'"') {
                        word.extend(std::iter::repeat_n('\\', n / 2));
                        if n % 2 == 1 {
                            chars.next();
                            word.push('"');
                        }
                        // Even: the quote is a quote, handled on the next iteration.
                    } else {
                        word.extend(std::iter::repeat_n('\\', n));
                    }
                    in_word = true;
                }
                '"' => {
                    if quoted && chars.next_if_eq(&'"').is_some() {
                        word.push('"');
                        quoted = false;
                    } else {
                        quoted = !quoted;
                    }
                    in_word = true;
                }
                ' ' | '\t' if !quoted => {
                    if in_word {
                        words.push(std::mem::take(&mut word));
                        in_word = false;
                    }
                }
                c => {
                    word.push(c);
                    in_word = true;
                }
            }
        }
        if in_word {
            words.push(word);
        }
        words
    }

    /// What `cmd.exe` passes on for a line typed at its prompt, `%` expansion aside: every `"`
    /// flips its quote state, and outside quotes `^` escapes the next character. Panics on an
    /// unescaped `& | < >` outside quotes, which cmd would run as an operator.
    pub(crate) fn cmd_unescape(line: &str) -> String {
        let mut out = String::with_capacity(line.len());
        let mut quoted = false;
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => quoted = !quoted,
                '^' if !quoted => {
                    out.extend(chars.next());
                    continue;
                }
                '&' | '|' | '<' | '>' if !quoted => {
                    panic!("cmd.exe would run {c:?} as an operator in {line}")
                }
                _ => {}
            }
            out.push(c);
        }
        out
    }

    /// The argv a Windows command line yields when pasted into `cmd.exe`.
    pub(crate) fn cmd_split(line: &str) -> Vec<String> {
        windows_split(&cmd_unescape(line))
    }

    #[test]
    fn windows_split_follows_command_line_to_argv_w() {
        let cases: [(&str, &[&str]); 9] = [
            (r#"a "b c" d"#, &["a", "b c", "d"]),
            (r#""" x"#, &["", "x"]),
            (r"a\\b \c\", &[r"a\\b", r"\c\"]),
            (r#"a\"b"#, &[r#"a"b"#]),
            (r#"a\\"b c""#, &[r"a\b c"]),
            (r#"a\\\"b"#, &[r#"a\"b"#]),
            // `""` inside quotes: a literal quote that also ends the quoted part.
            (r#""a""b c""#, &[r#"a"b"#, "c"]),
            (r#""a""" b"#, &[r#"a" b"#]),
            ("a\tb\n", &["a", "b\n"]),
        ];
        for (line, expected) in cases {
            assert_eq!(windows_split(line), expected, "{line}");
        }
    }

    #[test]
    fn current_matches_the_build_target() {
        let expected = if cfg!(windows) {
            HostPlatform::Windows
        } else {
            HostPlatform::Unix
        };
        assert_eq!(HostPlatform::current(), expected);
    }

    #[test]
    fn posix_quoting() {
        for (arg, expected) in POSIX_CASES {
            assert_eq!(quote_arg(arg, HostPlatform::Unix), expected, "{arg:?}");
        }
    }

    /// The real `sh` reads every quoted case back as the original argument.
    #[cfg(unix)]
    #[test]
    fn posix_quoting_round_trips_through_sh() {
        let line = POSIX_CASES
            .iter()
            .map(|(arg, _)| quote_arg(arg, HostPlatform::Unix))
            .collect::<Vec<_>>()
            .join(" ");
        let args: Vec<&str> = POSIX_CASES.iter().map(|(arg, _)| *arg).collect();
        assert_eq!(sh_split(&line), args);
    }

    /// (argument, POSIX-quoted form)
    const POSIX_CASES: [(&str, &str); 19] = [
        // Safe set: left bare.
        (
            "plain-value_1.2:3,4@5%6+7=8/9",
            "plain-value_1.2:3,4@5%6+7=8/9",
        ),
        ("", "''"),
        ("a b", "'a b'"),
        ("it's", r"'it'\''s'"),
        (r#"say "hi""#, r#"'say "hi"'"#),
        (r"C:\dir\", r"'C:\dir\'"),
        ("$HOME*", "'$HOME*'"),
        ("日本語", "'日本語'"),
        // zsh would expand a leading `=`.
        ("=ls", "'=ls'"),
        ("https://a.test/?x=1&y=2", "'https://a.test/?x=1&y=2'"),
        // Switches quote only their value.
        (
            "--user-data-dir=/tmp/my dir",
            "--user-data-dir='/tmp/my dir'",
        ),
        (
            "--user-data-dir=/home/me/配置 1",
            "--user-data-dir='/home/me/配置 1'",
        ),
        ("--window-name=it's", r"--window-name='it'\''s'"),
        (
            "--proxy-server=socks5://[::1]:1080",
            "--proxy-server='socks5://[::1]:1080'",
        ),
        ("--empty=", "--empty="),
        ("--a=b=c d", "--a='b=c d'"),
        ("--a==b", "--a='=b'"),
        // Not a plain switch name: the whole word is quoted.
        ("--odd name=v", "'--odd name=v'"),
        ("=v w", "'=v w'"),
    ];

    /// Splits a command line with the system `sh`, which prints each word it parsed.
    #[cfg(unix)]
    pub(crate) fn sh_split(line: &str) -> Vec<String> {
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(r"printf '%s\0' {line}"))
            .output()
            .expect("run sh");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("sh output is UTF-8");
        let mut words: Vec<String> = stdout.split('\0').map(str::to_owned).collect();
        // Every word ends with NUL, so the last piece is empty.
        assert_eq!(words.pop().as_deref(), Some(""));
        words
    }

    #[test]
    fn windows_quoting() {
        let cases = [
            ("plain-value_1.2", "plain-value_1.2"),
            ("", r#""""#),
            ("a b", r#""a b""#),
            // Single quotes mean nothing to CommandLineToArgvW.
            ("it's", "it's"),
            (r#"say "hi""#, r#""say \"hi\"""#),
            // Backslashes are literal unless they precede a quote.
            (r"C:\dir\", r"C:\dir\"),
            (r"C:\a\\b c", r#""C:\a\\b c""#),
            (r"C:\my dir\", r#""C:\my dir\\""#),
            (r#"a\"b"#, r#""a\\\"b""#),
            ("tab\tx", "\"tab\tx\""),
            ("https://a.test/?x=1&y=2", r#""https://a.test/?x=1&y=2""#),
            ("a|b<c>d^e", r#""a|b<c>d^e""#),
            ("日本語", "日本語"),
            // Switches quote only their value.
            (
                r"--user-data-dir=C:\Users\me\my dir\",
                r#"--user-data-dir="C:\Users\me\my dir\\""#,
            ),
            (r#"--window-name=say "hi""#, r#"--window-name="say \"hi\"""#),
            (
                "--proxy-server=socks5://[::1]:1080",
                "--proxy-server=socks5://[::1]:1080",
            ),
            ("--empty=", "--empty="),
        ];
        for (arg, expected) in cases {
            let quoted = quote_arg(arg, HostPlatform::Windows);
            assert_eq!(quoted, expected, "{arg:?}");
            assert_eq!(windows_split(&quoted), [arg], "{arg:?}");
            assert_eq!(cmd_split(&quoted), [arg], "{arg:?}");
        }
    }

    /// One embedded quote inverts cmd.exe's quote state for the rest of the line; carets keep
    /// the later metacharacters from running as operators.
    #[test]
    fn windows_line_survives_cmd_after_an_odd_quote() {
        let args = [
            r#"--window-name=say "hi"#.to_owned(),
            "https://a.test/?x=1&y=2".to_owned(),
            "a^b c".to_owned(),
            r#"--n="x" & y"#.to_owned(),
        ];
        let line = command_line(r"C:\Chromium\chrome.exe", &args, HostPlatform::Windows);
        assert_eq!(
            line,
            r#"C:\Chromium\chrome.exe --window-name="say \"hi" "https://a.test/?x=1^&y=2" "a^^b c" --n="\"x\" ^& y""#
        );
        let words = cmd_split(&line);
        assert_eq!(words[0], r"C:\Chromium\chrome.exe");
        assert_eq!(words[1..], args);
        // Balanced quotes: no carets.
        let args = ["https://a.test/?x=1&y=2".to_owned()];
        assert_eq!(
            command_line(r"C:\chrome.exe", &args, HostPlatform::Windows),
            r#"C:\chrome.exe "https://a.test/?x=1&y=2""#
        );
        // The rest of the same word is affected too.
        let args = [r#"--window-name=say "hi & bye"#.to_owned()];
        let line = command_line(r"C:\chrome.exe", &args, HostPlatform::Windows);
        assert_eq!(line, r#"C:\chrome.exe --window-name="say \"hi ^& bye""#);
        assert_eq!(cmd_split(&line)[1..], args);
    }

    #[test]
    fn command_line_quotes_program_and_args() {
        let args = [
            "--lang=zh-CN".to_owned(),
            "--x=a b".to_owned(),
            "".to_owned(),
        ];
        assert_eq!(
            command_line("/opt/My Apps/chrome", &args, HostPlatform::Unix),
            "'/opt/My Apps/chrome' --lang=zh-CN --x='a b' ''"
        );
        assert_eq!(
            command_line(
                r"C:\Program Files\Chromium\Application\chrome.exe",
                &args,
                HostPlatform::Windows
            ),
            r#""C:\Program Files\Chromium\Application\chrome.exe" --lang=zh-CN --x="a b" """#
        );
        assert_eq!(
            command_line("./chrome", &[], HostPlatform::Unix),
            "./chrome"
        );
    }
}
