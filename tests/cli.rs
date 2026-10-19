use std::process::Command;

#[test]
fn reports_the_packaged_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_red-table"))
        .arg("--version")
        .output()
        .expect("red-table binary should start");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version output should be UTF-8"),
        format!("red-table {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn documents_the_terminal_interface() {
    let output = Command::new(env!("CARGO_BIN_EXE_red-table"))
        .arg("--help")
        .output()
        .expect("red-table binary should start");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help output should be UTF-8");
    assert!(stdout.contains("terminal image browser"));
    assert!(stdout.contains("[PATH]"));
    assert!(stdout.contains("image to inspect"));
    assert!(stdout.contains("hjkl"));
    assert!(stdout.contains("--thumbnail-size"));
    assert!(stdout.contains("--quality"));
    assert!(stdout.contains("--graphics-protocol"));
    assert!(stdout.contains("--config"));
    assert!(stdout.contains("--no-config"));
    assert!(stdout.contains("--select"));
    assert!(stdout.contains("--print0"));
    assert!(stdout.contains("--files0-from"));
    assert!(stdout.contains("shell-init <bash|zsh|fish>"));
    assert_eq!(red_table::RunOutcome::Completed.exit_code(), 0);
    assert_eq!(red_table::RunOutcome::Cancelled.exit_code(), 2);
}

#[test]
fn rejects_an_invalid_configuration_before_starting_the_terminal() {
    let path = std::env::temp_dir().join(format!(
        "red-table-invalid-config-{}-{}.toml",
        std::process::id(),
        env!("CARGO_PKG_VERSION")
    ));
    std::fs::write(&path, "version = 1\nunknown = true\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_red-table"))
        .arg("--config")
        .arg(&path)
        .output()
        .expect("red-table binary should start");
    std::fs::remove_file(path).unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("error output should be UTF-8");
    assert!(stderr.contains("invalid configuration"));
    assert!(stderr.contains("unknown field"));
}

#[test]
fn rejects_a_missing_directory_before_starting_the_terminal() {
    let missing = std::env::temp_dir().join(format!(
        "red-table-missing-{}-{}",
        std::process::id(),
        env!("CARGO_PKG_VERSION")
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_red-table"))
        .arg(&missing)
        .output()
        .expect("red-table binary should start");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("error output should be UTF-8");
    assert!(stderr.contains("cannot open"));
    assert!(stderr.starts_with("red-table: error:\n"));
    assert!(stderr.contains("hint: Check that the path exists"));
    assert!(!stderr.contains('\x1b'));
}

#[test]
fn redirected_errors_stay_plain_even_with_a_color_capable_term() {
    for no_color in [None, Some("1"), Some("")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_red-table"));
        command
            .args(["--no-config", "--unknown-option"])
            .env("TERM", "xterm-256color");
        if let Some(value) = no_color {
            command.env("NO_COLOR", value);
        } else {
            command.env_remove("NO_COLOR");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.starts_with("red-table: error:\n  unknown option:"));
        assert!(stderr.contains("hint: Run `red-table --help`"));
        assert!(!stderr.contains('\x1b'));
    }
}

#[test]
fn rejects_unsupported_files_with_a_format_hint_before_terminal_initialization() {
    let path =
        std::env::temp_dir().join(format!("red-table-unsupported-{}.txt", std::process::id()));
    std::fs::write(&path, b"notes").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_red-table"))
        .arg("--no-config")
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("not a supported image or directory"));
    assert!(stderr.contains("hint: Pass a directory or a JPEG, PNG"));
    assert!(!stderr.contains('\x1b'));
}

#[test]
fn rejects_an_invalid_thumbnail_size_before_starting_the_terminal() {
    let output = Command::new(env!("CARGO_BIN_EXE_red-table"))
        .args(["--thumbnail-size", "5x2"])
        .output()
        .expect("red-table binary should start");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("error output should be UTF-8");
    assert!(stderr.contains("between 12x6 and 120x60"));
}

#[test]
fn rejects_an_invalid_quality_before_starting_the_terminal() {
    let output = Command::new(env!("CARGO_BIN_EXE_red-table"))
        .args(["--quality", "10"])
        .output()
        .expect("red-table binary should start");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("error output should be UTF-8");
    assert!(stderr.contains("level from 1 to 9"));
}

#[test]
fn rejects_an_invalid_graphics_protocol_before_starting_the_terminal() {
    let output = Command::new(env!("CARGO_BIN_EXE_red-table"))
        .args(["--graphics-protocol", "ansi"])
        .output()
        .expect("red-table binary should start");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("error output should be UTF-8");
    assert!(stderr.contains("auto, kitty, sixel, iterm2, or halfblocks"));
}

#[test]
fn rejects_result_delimiter_without_selection_before_starting_the_terminal() {
    let output = Command::new(env!("CARGO_BIN_EXE_red-table"))
        .arg("--print0")
        .output()
        .expect("red-table binary should start");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("error output should be UTF-8");
    assert!(stderr.contains("--print0 requires --select"));
}

#[test]
fn rejects_malformed_nul_input_before_starting_the_terminal() {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_red-table"))
        .arg("--files0-from=-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("red-table binary should start");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"unterminated.jpg")
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("error output should be UTF-8");
    assert!(stderr.contains("ends without a NUL delimiter"));
}
