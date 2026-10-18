# Configuration

`red-table` loads optional versioned TOML configuration from
`$XDG_CONFIG_HOME/red-table/config.toml`. If `XDG_CONFIG_HOME` is empty or
unset, it uses `$HOME/.config/red-table/config.toml`. A missing discovered file
is harmless.

Use `--config PATH` to require one specific file or `--no-config` to disable
discovery. Command-line values override TOML values, and TOML values override
the built-in defaults. Unknown fields, unsupported versions, invalid values,
unknown actions, malformed key names, and two actions sharing one key in the
same context are startup errors. This intentionally prevents silent typos.

The complete version 1 schema with built-in values is:

```toml
version = 1

[thumbnail]
size = "32x14"
quality = 7

[graphics]
protocol = "auto" # auto, kitty, sixel, iterm2, or halfblocks

[ui]
debug_status = false

[inspection]
background = "checkerboard" # checkerboard, dark, or light
decoded_cache_mib = 512      # 64 through 2048

[keys.normal]
toggle_sort = ["s"]
quit = ["q"]
inspect = ["Enter"]
compare = ["c"]
toggle_mark = ["Space"]
mark_range = ["v"]
clear_marks = ["u"]
marked_only = ["m"]
confirm_selection = ["Ctrl+s"]
search = ["/"]
thumbnail_larger = ["+", "="]
thumbnail_smaller = ["-"]
thumbnail_reset = ["0"]
quality_1 = ["1"]
quality_2 = ["2"]
quality_3 = ["3"]
quality_4 = ["4"]
quality_5 = ["5"]
quality_6 = ["6"]
quality_7 = ["7"]
quality_8 = ["8"]
quality_9 = ["9"]
left = ["Left", "h"]
right = ["Right", "l"]
up = ["Up", "k"]
down = ["Down", "j"]
page_up = ["PageUp"]
page_down = ["PageDown"]
first = ["Home"]
last = ["End"]
help = ["?"]
debug = ["F12"]

[keys.search]
cancel = ["Esc"]
confirm = ["Enter"]
backspace = ["Backspace"]

[keys.inspect]
quit = ["q"]
close = ["Esc"]
previous = ["p"]
next = ["n"]
fit_100 = ["z"]
zoom_in = ["+"]
zoom_out = ["-"]
pan_left = ["Left", "h"]
pan_right = ["Right", "l"]
pan_up = ["Up", "k"]
pan_down = ["Down", "j"]
background = ["b"]
help = ["?"]
debug = ["F12"]
toggle_mark = ["Space"]
mark_range = ["v"]
clear_marks = ["u"]
marked_only = ["m"]
confirm_selection = ["Ctrl+s"]

[keys.compare]
quit = ["q"]
close = ["Esc"]
previous = ["p"]
next = ["n"]
promote = ["Enter"]
switch_pane = ["Tab"]
sync = ["s"]
fit_100 = ["z"]
zoom_in = ["+"]
zoom_out = ["-"]
pan_left = ["Left", "h"]
pan_right = ["Right", "l"]
pan_up = ["Up", "k"]
pan_down = ["Down", "j"]
background = ["b"]
toggle_mark = ["Space"]
clear_marks = ["u"]
marked_only = ["m"]
help = ["?"]
debug = ["F12"]
confirm_selection = ["Ctrl+s"]
```

Sections and values may be omitted. A configured action replaces all of that
action's built-in keys; actions not mentioned retain their defaults. Reusing a
key in different contexts is valid.

Key names are case-sensitive. Printable characters can be written directly.
Named keys include `Enter`, `Esc`, `Space`, `Backspace`, arrows, `PageUp`,
`PageDown`, `Home`, `End`, `Tab`, and `F1` through `F12`. Prefix modifiers with
`Ctrl+`, `Alt+`, or `Shift+`, for example `Ctrl+f` or `Shift+x`. `Ctrl+c` is
reserved as an unconfigurable emergency interrupt.

Press the configured `help` key to see the bindings effective in the current
interaction context. The normal status stays concise; the configured `debug`
action reveals renderer, quality, queue, and cache diagnostics.

The decoded inspection cache is byte-bounded independently from the thumbnail
cache. Its value is in mebibytes. Transparent images are composited onto the
configured initial background; `background` cycles the three modes at runtime.

Selection actions are available in normal and inspection contexts. The
`confirm_selection` action ends only a `--select` session; it is inert during a
normal browsing session. `Ctrl+c` remains reserved and cannot be rebound.

Comparison bindings live in their own context. Enter promotes the candidate
instead of opening inspection, while Tab selects which pane receives independent
zoom, pan, and Space marking. Enabling `sync` copies the active view state to the
other pane and couples subsequent geometry changes.
