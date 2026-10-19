//! Run generated integration with real interpreters, isolated startup files,
//! and a fake picker. No personal shell configuration or interactive TTY needed.
#![cfg(unix)]

use std::{
    fs,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn checked_expect(script: &str) -> String {
    format!("if {{[catch {{\n{script}\n}} message]}} {{puts stderr $message; exit 1}}\n")
}
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new(shell: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "red-table-shell-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir_all(root.join("broken-config/red-table")).unwrap();
        fs::write(
            root.join("broken-config/red-table/config.toml"),
            "invalid = [",
        )
        .unwrap();
        let fixture = Self { root };
        let root = &fixture.root;
        let generated = fixture
            .command(env!("CARGO_BIN_EXE_red-table"))
            .args(["shell-init", shell])
            .env("XDG_CONFIG_HOME", root.join("broken-config"))
            .output()
            .unwrap();
        assert!(generated.status.success(), "{generated:?}");
        assert!(generated.stderr.is_empty());
        fs::write(root.join("init"), generated.stdout).unwrap();
        let bash =
            std::env::split_paths(&std::env::var_os("PATH").expect("flake must supply PATH"))
                .map(|directory| directory.join("bash"))
                .find(|path| path.is_file())
                .expect("flake must supply Bash")
                .canonicalize()
                .unwrap();
        fs::write(root.join("red-table"), format!("#!{}\n[[ $# == 2 && $1 == --select && $2 == --print0 ]] || exit 64\nprintf called > \"$TEST_ROOT/called\"\ncat -- \"$TEST_ROOT/paths\"\nexit \"$TEST_STATUS\"\n", bash.display())).unwrap();
        fs::set_permissions(root.join("red-table"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("paths"), b"").unwrap();
        fixture
    }
    fn command(&self, executable: &str) -> Command {
        let mut command = Command::new(executable);
        command
            .env_clear()
            .current_dir(&self.root)
            .env("HOME", &self.root)
            .env("ZDOTDIR", &self.root)
            .env("TMPDIR", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("TEST_ROOT", &self.root)
            .env("LC_ALL", "C")
            .env("BASH_ENV", "/dev/null")
            .env("ENV", "/dev/null")
            // NixOS global startup must not replace this fixture's PATH.
            .env("__NIXOS_SET_ENVIRONMENT_DONE", "1")
            .env("NOSYSBASHRC", "1")
            .env(
                "PATH",
                format!("{}:{}", self.root.display(), std::env::var("PATH").unwrap()),
            );
        command
    }

    fn run(&self, shell: &str, code: &str, status: &str, prefix: &str) -> Output {
        let mut command = self.command(shell);
        match shell {
            "bash" => {
                command.args(["--noprofile", "--norc"]);
            }
            "zsh" => {
                command.arg("-f");
            }
            "fish" => {
                command.arg("--no-config");
            }
            _ => unreachable!(),
        }
        command
            .args(["-c", code])
            .env("TEST_STATUS", status)
            .env("TEST_PREFIX", prefix)
            .env("TEST_SUFFIX", " 'tail'")
            .env("TEST_POINT", prefix.len().to_string())
            .output()
            .unwrap()
    }
    fn assert_no_temporary_files(&self) {
        for entry in fs::read_dir(&self.root).unwrap() {
            assert!(
                !entry
                    .unwrap()
                    .file_name()
                    .as_bytes()
                    .starts_with(b"red-table-shell.")
            );
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

const BASH_PICK: &str = r#"
source "$TEST_ROOT/init"
READLINE_LINE=$TEST_PREFIX$TEST_SUFFIX
READLINE_POINT=$TEST_POINT
_red_table_pick
if [[ $TEST_STATUS == 0 && -s $TEST_ROOT/paths ]]; then
    eval "$READLINE_LINE"
else
    printf '%s\0%s' "$READLINE_LINE" "$READLINE_POINT"
fi
"#;
const ZSH_PICK: &str = r#"
source "$TEST_ROOT/init"
LBUFFER=$TEST_PREFIX RBUFFER=$TEST_SUFFIX
_red_table_pick
if [[ $TEST_STATUS == 0 && -s $TEST_ROOT/paths ]]; then
    eval "$LBUFFER$RBUFFER"
else
    print -rn -- "$LBUFFER$RBUFFER"
fi
"#;
const FISH_PICK: &str = r#"
source "$TEST_ROOT/init"
set -g test_line "$TEST_PREFIX$TEST_SUFFIX"
function commandline
    switch $argv[1]
        case --cut-at-cursor
            printf '%s\n' "$TEST_PREFIX"
        case --insert
            set -g test_line "$TEST_PREFIX$argv[3]$TEST_SUFFIX"
        case -f
            return 0
        case '*'
            return 64
    end
end
_red_table_pick
if test "$TEST_STATUS" = 0; and test -s "$TEST_ROOT/paths"
    eval "$test_line"
else
    printf '%s' "$test_line"
end
"#;

#[test]
fn shells_resolve_the_fixture_picker_without_host_startup_overrides() {
    for shell in ["bash", "zsh", "fish"] {
        let fixture = Fixture::new(shell);
        let output = fixture.run(
            shell,
            "source \"$TEST_ROOT/init\"; command -v red-table",
            "0",
            "",
        );
        assert!(output.status.success(), "{shell}: {output:?}");
        assert!(output.stderr.is_empty(), "{shell}: {output:?}");
        assert_eq!(
            output.stdout,
            format!("{}\n", fixture.root.join("red-table").display()).as_bytes(),
            "{shell}: shell startup replaced the fixture executable search path"
        );
    }
}

#[test]
fn widgets_quote_arbitrary_filename_bytes_without_executing_them() {
    for (shell, code) in [("bash", BASH_PICK), ("zsh", ZSH_PICK), ("fish", FISH_PICK)] {
        let fixture = Fixture::new(shell);
        let paths: Vec<&[u8]> = vec![
            b"/photos/space and 'single' \"double\".png",
            b"/photos/line\nbreak\n.png",
            b"/photos/$(touch INJECTED);`touch INJECTED`\\*.png",
            b"/photos/utf8-\xc3\xa4-raw-\xff.png",
            b"/photos/trailing-newline.png\n",
        ];
        let mut expected = "Grüße\0".as_bytes().to_vec();
        let mut bytes = Vec::new();
        for path in paths {
            bytes.extend_from_slice(path);
            bytes.push(0);
        }
        fs::write(fixture.root.join("paths"), &bytes).unwrap();
        expected.extend_from_slice(&bytes);
        expected.extend_from_slice(b"tail\0");
        let output = fixture.run(shell, code, "0", "printf '%s\\0' 'Grüße' ");
        assert!(output.status.success(), "{shell}: {output:?}");
        assert!(
            output.stderr.is_empty(),
            "{shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected, "{shell}");
        assert!(!fixture.root.join("INJECTED").exists());
        fixture.assert_no_temporary_files();
    }
}

#[test]
fn widgets_preserve_the_buffer_on_cancel_failure_empty_results_and_unfinished_quotes() {
    for (shell, code) in [("bash", BASH_PICK), ("zsh", ZSH_PICK), ("fish", FISH_PICK)] {
        for (status, bytes, prefix, called) in [
            ("2", b"/partial.png\0".as_slice(), "echo ", true),
            ("1", b"/partial.png\0".as_slice(), "echo ", true),
            ("0", b"".as_slice(), "echo ", true),
            ("2", b"/partial.png\0".as_slice(), "echo 'unfinished", false),
            (
                "2",
                b"/partial.png\0".as_slice(),
                "echo \"unfinished",
                false,
            ),
        ] {
            let fixture = Fixture::new(shell);
            fs::write(fixture.root.join("paths"), bytes).unwrap();
            let output = fixture.run(shell, code, status, prefix);
            assert!(output.status.success(), "{shell}: {output:?}");
            let mut expected = format!("{prefix} 'tail'").into_bytes();
            if shell == "bash" {
                expected.extend_from_slice(format!("\0{}", prefix.len()).as_bytes());
            }
            assert_eq!(output.stdout, expected, "{shell}: {output:?}");
            assert!(output.stderr.is_empty(), "{shell}: {output:?}");
            assert_eq!(fixture.root.join("called").exists(), called, "{shell}");
            fixture.assert_no_temporary_files();
        }
    }
}

#[test]
fn bash_and_fish_complete_options_values_and_shell_names() {
    for (shell, code) in [
        (
            "bash",
            r#"
source "$TEST_ROOT/init"
COMP_WORDS=(red-table --graphics-protocol ki); COMP_CWORD=2
_red_table_complete; printf '%s\n' "${COMPREPLY[@]}"
COMP_WORDS=(red-table --quality=9); COMP_CWORD=1
_red_table_complete; printf '%s\n' "${COMPREPLY[@]}"
COMP_WORDS=(red-table shell-init f); COMP_CWORD=2
_red_table_complete; printf '%s\n' "${COMPREPLY[@]}"
COMP_WORDS=(red-table --no-c); COMP_CWORD=1
_red_table_complete; printf '%s\n' "${COMPREPLY[@]}"
"#,
        ),
        (
            "fish",
            r#"
source "$TEST_ROOT/init"
complete -C 'red-table --graphics-protocol ki' | string split -f 1 \t
complete -C 'red-table --quality=9' | string split -f 1 \t
complete -C 'red-table shell-init f' | string split -f 1 \t
complete -C 'red-table --no-c' | string split -f 1 \t
"#,
        ),
    ] {
        let fixture = Fixture::new(shell);
        let output = fixture.run(shell, code, "0", "");
        assert!(output.status.success(), "{shell}: {output:?}");
        assert!(output.stderr.is_empty(), "{shell}: {output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "kitty\n--quality=9\nfish\n--no-config\n",
            "{shell}"
        );
    }
}

#[test]
fn shell_init_rejects_invalid_requests_without_partial_scripts() {
    for arguments in [
        vec!["shell-init"],
        vec!["shell-init", "powershell"],
        vec!["shell-init", "bash", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_red-table"))
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("bash, zsh, or fish")
        );
    }
}

#[test]
fn zsh_completion_and_alt_i_work_in_the_real_line_editor() {
    let fixture = Fixture::new("zsh");
    fs::write(fixture.root.join("settings with spaces.toml"), b"").unwrap();
    fs::write(fixture.root.join("paths"), b"/photos/a b.png\0").unwrap();
    fs::write(
        fixture.root.join(".zshrc"),
        r#"
autoload -Uz compinit
compinit -D
source "$TEST_ROOT/init"
_red_table_test_dump() {
    print -rn -- "$BUFFER" > "$TEST_ROOT/buffer-next"
    command mv -- "$TEST_ROOT/buffer-next" "$TEST_ROOT/buffer"
    BUFFER=
    zle reset-prompt
}
zle -N _red_table_test_dump
bindkey '^X^B' _red_table_test_dump
PS1='RED_TABLE_READY> '
RPS1=
"#,
    )
    .unwrap();
    let script = r#"
log_user 0
set timeout 10
set stty_init "rows 24 columns 120"
spawn -noecho zsh -d -i
expect_before timeout {error "Zsh line editor timed out"}
expect -exact "RED_TABLE_READY> "
proc wait_for_file {path} {
    for {set count 0} {$count < 250} {incr count} {
        if {[file exists $path]} {return}
        after 20
    }
    error "Zsh did not create $path"
}
foreach {input expected} {
    {red-table --graphics-protocol ki} {red-table --graphics-protocol kitty}
    {red-table --quality=9} {red-table --quality=9}
    {red-table shell-init f} {red-table shell-init fish}
    {red-table --config sett} {red-table --config settings\ with\ spaces.toml}
    {red-table sett} {red-table settings\ with\ spaces.toml}
} {
    send -- "$input\t"
    after 150
    send -- "\030\002"
    wait_for_file "$env(TEST_ROOT)/buffer"
    set file [open "$env(TEST_ROOT)/buffer"]
    set actual [string trimright [read $file]]
    close $file
    file delete "$env(TEST_ROOT)/buffer"
    if {$actual ne $expected} {error "completion mismatch: <$actual>, expected <$expected>"}
}
send -- "echo "
send -- "\033i"
after 200
send -- "\030\002"
wait_for_file "$env(TEST_ROOT)/buffer"
set file [open "$env(TEST_ROOT)/buffer"]
set actual [string trimright [read $file]]
close $file
if {$actual ne {echo  /photos/a\ b.png}} {error "Alt+i did not insert the selected path: <$actual>"}
send -- "exit\r"
expect eof
set result [wait]
exit [lindex $result 3]
"#;
    let output = fixture
        .command("expect")
        .args(["-c", &checked_expect(script)])
        .env("TEST_STATUS", "0")
        .env("TERM", "xterm-256color")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fixture.assert_no_temporary_files();
}

#[test]
fn bash_and_fish_alt_i_bindings_insert_into_the_real_command_buffer() {
    for shell in ["bash", "fish"] {
        let fixture = Fixture::new(shell);
        fs::write(fixture.root.join("settings with spaces.toml"), b"").unwrap();
        fs::write(fixture.root.join("paths"), b"/photos/a b.png\0").unwrap();
        let init = if shell == "bash" {
            r#"
source "$TEST_ROOT/init"
_red_table_test_dump() {
    printf '%s' "$READLINE_LINE" > "$TEST_ROOT/buffer-next"
    command mv -- "$TEST_ROOT/buffer-next" "$TEST_ROOT/buffer"
    READLINE_LINE=
    READLINE_POINT=0
}
bind -x '"\C-x\C-b":_red_table_test_dump'
PS1='RED_TABLE_READY> '
"#
        } else {
            r#"
source "$TEST_ROOT/init"
function fish_prompt
    printf 'RED_TABLE_READY> '
end
function _red_table_test_dump
    commandline | string collect --no-trim-newlines > "$TEST_ROOT/buffer-next"
    command mv -- "$TEST_ROOT/buffer-next" "$TEST_ROOT/buffer"
    commandline --replace ''
    commandline -f repaint
end
bind \cx\cb _red_table_test_dump
"#
        };
        fs::write(
            fixture.root.join("interactive"),
            format!("{init}\nprintf ready > \"$TEST_ROOT/ready\"\n"),
        )
        .unwrap();
        let script = r#"
log_user 0
set timeout 10
set stty_init "rows 24 columns 120"
proc wait_for_file {path} {
    for {set count 0} {$count < 250} {incr count} {
        if {[file exists $path]} {return}
        after 20
    }
    error "shell did not create $path"
}
if {$env(TEST_SHELL) eq "bash"} {
    spawn -noecho bash --noprofile --rcfile "$env(TEST_ROOT)/interactive" -i
} else {
    spawn -noecho fish --no-config -i -C {source "$TEST_ROOT/interactive"}
}
wait_for_file "$env(TEST_ROOT)/ready"
expect {
    -re {\x1b\[0?c} {send -- "\033\[?1;2c"; exp_continue}
    -exact "\033\[5n" {send -- "\033\[0n"; exp_continue}
    -exact "\033\[6n" {send -- "\033\[1;1R"; exp_continue}
    -re {RED_TABLE_READY[^\r\n]*> } {}
    timeout {error "shell prompt timed out"}
}
send -- "red-table --config sett\t"
after 200
send -- "\030\002"
wait_for_file "$env(TEST_ROOT)/buffer"
file rename "$env(TEST_ROOT)/buffer" "$env(TEST_ROOT)/completion-buffer"
send -- "printf '%s\\0' "
send -- "\033i"
after 250
send -- "\030\002"
wait_for_file "$env(TEST_ROOT)/buffer"
send -- "exit\r"
expect { eof {} timeout {error "shell did not exit"} }
set result [wait]
exit [lindex $result 3]
"#;
        let output = fixture
            .command("expect")
            .args(["-c", &checked_expect(script)])
            .env("TEST_SHELL", shell)
            .env("TEST_STATUS", "0")
            .env("TERM", "xterm-256color")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{shell}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let code = if shell == "bash" {
            r#"eval "$(cat "$TEST_ROOT/buffer")""#
        } else {
            r#"eval (cat "$TEST_ROOT/buffer" | string collect)"#
        };
        let output = fixture.run(shell, code, "0", "");
        assert!(output.status.success(), "{shell}: {output:?}");
        assert_eq!(output.stdout, b"/photos/a b.png\0", "{shell}: {output:?}");
        let completion_code = if shell == "bash" {
            r#"red-table() { printf '%s\0' "$@"; }; eval "$(cat "$TEST_ROOT/completion-buffer")""#
        } else {
            r#"function red-table; printf '%s\0' $argv; end; eval (cat "$TEST_ROOT/completion-buffer" | string collect)"#
        };
        let output = fixture.run(shell, completion_code, "0", "");
        assert!(output.status.success(), "{shell}: {output:?}");
        assert_eq!(
            output.stdout, b"--config\0settings with spaces.toml\0",
            "{shell}: {output:?}"
        );
        fixture.assert_no_temporary_files();
    }
}
