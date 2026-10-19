//! Human-readable CLI diagnostics. Call after restoring any terminal session.

use std::{
    error::Error,
    ffi::OsStr,
    fmt,
    io::{self, Write},
};

#[derive(Debug)]
pub(crate) struct Diagnostic {
    message: String,
    hint: &'static str,
}

impl Diagnostic {
    pub(crate) fn new(message: String, hint: &'static str) -> Self {
        Self { message, hint }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for Diagnostic {}

/// Color belongs to the destination stream and respects explicit opt-out.
pub fn use_color(is_terminal: bool, term: Option<&OsStr>, no_color: Option<&OsStr>) -> bool {
    is_terminal
        && term.is_some_and(|value| !value.is_empty() && value != "dumb")
        && no_color.is_none_or(OsStr::is_empty)
}

/// Write a diagnostic without using stdout or changing the error's exit status.
pub fn write_error(
    writer: &mut impl Write,
    error: &(dyn Error + 'static),
    color: bool,
) -> io::Result<()> {
    let (error_label, hint_label) = if color {
        ("\x1b[1;31merror\x1b[0m", "\x1b[36mhint\x1b[0m")
    } else {
        ("error", "hint")
    };
    writeln!(writer, "red-table: {error_label}:")?;
    for line in error.to_string().lines() {
        writeln!(writer, "  {line}")?;
    }
    let mut source = error.source();
    while let Some(cause) = source {
        writeln!(writer, "  caused by:")?;
        for line in cause.to_string().lines() {
            writeln!(writer, "    {line}")?;
        }
        source = cause.source();
    }
    let hint = error.downcast_ref::<Diagnostic>().map_or(
        "Run `red-table --help` for usage and available options.",
        |diagnostic| diagnostic.hint,
    );
    writeln!(writer, "\n  {hint_label}: {hint}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_requires_a_capable_terminal_and_respects_no_color() {
        for (tty, term, no_color, expected) in [
            (true, Some("xterm-256color"), None, true),
            (true, Some("xterm"), Some(""), true),
            (true, Some("xterm"), Some("0"), false),
            (false, Some("xterm"), None, false),
            (true, Some("dumb"), None, false),
            (true, Some(""), None, false),
            (true, None, None, false),
        ] {
            assert_eq!(
                use_color(tty, term.map(OsStr::new), no_color.map(OsStr::new)),
                expected
            );
        }
    }

    #[test]
    fn multiline_errors_and_causes_are_indented_with_recovery_guidance() {
        let error = io::Error::other(Diagnostic::new("bad config\nline 2".into(), "Fix it."));
        let mut bytes = Vec::new();
        write_error(&mut bytes, &error, false).unwrap();
        let output = String::from_utf8(bytes).unwrap();
        assert!(output.starts_with("red-table: error:\n  bad config\n  line 2\n"));
        assert!(output.contains("hint: Run `red-table --help`"));
        assert!(!output.contains('\x1b'));
    }

    #[test]
    fn color_changes_only_labels_and_preserves_the_specific_hint() {
        let error = Diagnostic::new("cannot open photos".into(), "Check the path.");
        let mut bytes = Vec::new();
        write_error(&mut bytes, &error, true).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "red-table: \x1b[1;31merror\x1b[0m:\n  cannot open photos\n\n  \x1b[36mhint\x1b[0m: Check the path.\n"
        );
    }
}
