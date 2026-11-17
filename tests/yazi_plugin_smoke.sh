#!/usr/bin/env bash
set -euo pipefail

smoke_root=$(mktemp -d -t red-table-yazi-smoke.XXXXXXXX)
cleanup() {
  if [[ -n "$smoke_root" && -d "$smoke_root" ]]; then
    rm -rf -- "$smoke_root"
  fi
}
trap cleanup EXIT

mkdir -p "$smoke_root/config/plugins" "$smoke_root/images" "$smoke_root/state"
cp -R red-table.yazi "$smoke_root/config/plugins/red-table.yazi"
install -m 0644 tests/yazi-smoke/init.lua "$smoke_root/config/init.lua"
install -m 0644 tests/yazi-smoke/keymap.toml "$smoke_root/config/keymap.toml"
install -m 0755 tests/yazi-smoke/red-table-wrapper.sh "$smoke_root/red-table-wrapper"

cargo build --quiet --bin red-table

smoke_yazi=${RED_TABLE_SMOKE_YAZI:-}
if [[ -z "$smoke_yazi" ]]; then
  smoke_yazi=$(command -v yazi)
fi
test -x "$smoke_yazi"

export RED_TABLE_SMOKE_BINARY="$(realpath -- "${CARGO_TARGET_DIR:-target}/debug/red-table")"
export RED_TABLE_SMOKE_COMMAND="$smoke_root/red-table-wrapper"
export RED_TABLE_SMOKE_DIR="$smoke_root/images"
export RED_TABLE_SMOKE_MARKER="$smoke_root/started"
export RED_TABLE_SMOKE_STATUS="$smoke_root/status"
export RED_TABLE_SMOKE_YAZI="$smoke_yazi"
export RED_TABLE_SMOKE_LOG_USER=${RED_TABLE_SMOKE_LOG_USER:-0}
export XDG_STATE_HOME="$smoke_root/state"
export YAZI_CONFIG_HOME="$smoke_root/config"
export TERM="xterm-256color"
export LINES=24
export COLUMNS=80

expect <<'EXPECT'
log_user $env(RED_TABLE_SMOKE_LOG_USER)
set timeout 15
set stty_init "rows 24 columns 80"
spawn -noecho $env(RED_TABLE_SMOKE_YAZI) $env(RED_TABLE_SMOKE_DIR)
proc fail_smoke {message} {
  global env
  append message " (picker started: [file exists $env(RED_TABLE_SMOKE_MARKER)], status written: [file exists $env(RED_TABLE_SMOKE_STATUS)])"
  set child [exp_pid]
  catch {exec bash -c {kill -TERM -- "-$1" 2>/dev/null || kill -TERM "$1"} bash $child}
  after 200
  catch {exec bash -c {kill -KILL -- "-$1" 2>/dev/null || kill -KILL "$1"} bash $child}
  catch {wait}
  error $message
}
expect_before timeout {fail_smoke "Yazi smoke test timed out waiting for terminal readiness or shutdown"}
expect -exact "Yazi: images"
expect -exact "0/0"
after 200
send -- "R"
expect -exact "No supported images found"
send -- "\023"
expect -exact "Yazi: images"
expect -exact "0/0"
after 200
send -- "q"
set timeout 15
expect {
  eof {}
  -exact "Quit?" {
    send -- "y"
    expect eof
  }
}
set result [wait]
exit [lindex $result 3]
EXPECT

test -f "$RED_TABLE_SMOKE_MARKER"
test "$(<"$RED_TABLE_SMOKE_STATUS")" = "0"
printf 'Yazi PTY smoke test passed with %s\n' "$RED_TABLE_SMOKE_YAZI"
