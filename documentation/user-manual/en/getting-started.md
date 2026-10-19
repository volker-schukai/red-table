# Getting started

Open a directory recursively:

```sh
nix run . -- /path/to/photos
```

The default target is `32x14` terminal cells per thumbnail. Override it with:

```sh
nix run . -- --thumbnail-size 40x18 /path/to/photos
```

Accepted sizes range from `12x6` to `120x60`. They are target cell dimensions;
the grid distributes remaining terminal space evenly.

Images initially appear in discovery order. New discoveries append to the grid;
existing images keep their positions, including when the scan finishes. A
`--files0-from` list starts in its input order. Press `s` in the grid to toggle
between discovery order and byte-wise ascending relative path order. Focus and
marks stay with their images. In path mode, new discoveries are inserted into
path order; press `s` again to restore discovery order. The status shows the
current order. Each new session starts in discovery order.

Quality defaults to `7`. Set a startup level from `1` (fast) to `9` (maximum
detail enhancement) with:

```sh
nix run . -- --quality 9 /path/to/photos
```

Graphics default to automatic terminal negotiation. Override the result for
diagnosis or recovery with one of `auto`, `kitty`, `sixel`, `iterm2`, or
`halfblocks`:

```sh
nix run . -- --graphics-protocol kitty /path/to/photos
```

Without a path, `red-table` opens the current directory. It discovers JPEG, PNG,
GIF, WebP, TIFF, and BMP files incrementally, so the first results can appear
before a large directory has been scanned completely.

You can also pass an image directly:

```sh
red-table /path/to/photos/image.png
```

The image opens immediately in fullscreen inspection. Its parent directory is
scanned recursively in the background. Press `Esc` to see the collection with
that image focused, or `p`/`n` to inspect other discovered images. The requested
image appears first in discovery order; `s` in the grid switches to path order.
Symbolic links open their target image and its parent directory.

CLI errors appear on stderr with an error label and recovery hint. Labels use
color when stderr is a capable terminal. Redirected stderr, an unset or empty
`TERM`, `TERM=dumb`, or nonempty `NO_COLOR` disable diagnostic color.

## Configuration

Optional strict TOML configuration is discovered at
`$XDG_CONFIG_HOME/red-table/config.toml`, falling back to
`$HOME/.config/red-table/config.toml`. Use `--config PATH` to require a specific
file or `--no-config` to ignore discovery. CLI values take precedence. See the
[configuration reference](../../references/configuration.md) for the complete
version 1 schema and configurable action names.

## Keyboard controls

| Key | Action |
| --- | --- |
| arrows or `h`, `j`, `k`, `l` | move the selection |
| Page Up / Page Down | move by one viewport |
| Home / End | select the first / last match |
| Enter | inspect the focused image fullscreen |
| `c` | compare the focused image against candidates |
| `s` | toggle discovery / path order |
| Space | toggle a persistent `[x]` mark and double border on the focused image |
| `v` | mark the inclusive range from the last Space anchor |
| `u` / `m` | clear all marks / show only marked images |
| Ctrl+s | confirm marks in a `--select` session |
| `+` / `-` | enlarge / shrink thumbnail cells |
| `0` | restore the `32x14` default |
| `1` through `9` | set thumbnail quality immediately |
| `/` | start editing the search filter |
| `?` | show contextual help generated from active bindings |
| F12 | toggle technical renderer, quality, queue, and cache status |
| Enter while searching | keep the current filter |
| Escape while searching | restore the previous filter |
| `q` | quit and restore the terminal |

## Fullscreen inspection

Escape returns from inspection to the same focused tile and scroll position.
Use `p` and `n` to move through the current filtered ordering without returning
to the grid. `z` switches directly between fitting the complete image and exact
100% renderer samples; `+` and `-` step through 25, 50, 100, 200, 400, and 800%.
At a fixed zoom, arrows or `h`, `j`, `k`, and `l` pan in bounded increments.
Press `b` to inspect transparency against checkerboard, dark, and light
backgrounds.

EXIF orientation is applied consistently to thumbnails and inspection. Large or
corrupt sources show a recoverable error view. Inspection decode, zoom, and pan
run through their own bounded background pipeline, so input does not wait for a
source image and obsolete rapid-navigation work is discarded.

## A/B comparison

Press `c` with at least two visible images. The focused image becomes the pinned
left `REFERENCE`; the right `CANDIDATE` starts at the next visible image, or the
previous one when the reference was last. `p` and `n` cycle candidates while
always skipping the reference. Enter promotes the candidate to reference and
continues with another valid candidate. With exactly two images, the former
reference becomes the new candidate.

The red border identifies the active pane in addition to the literal role label.
Tab changes the active pane and Space toggles that image's `[x]` mark. Press `s`
to switch between synchronized and independent inspection. In synchronized mode,
fit/zoom and normalized pan are copied to both panes; enabling it copies the
active pane's current state. In independent mode, `z`, `+`, `-`, and the pan keys
change only the active side. Each pane loads and reports errors independently,
while both workers share the single configured decoded-memory budget. Escape
returns to the grid with the candidate focused.

## Visual file selection

Start a result session with `--select`. Focus remains the red bold border;
selection uses a yellow double border plus the separate literal `[x]`. A
focused mark therefore has a red bold double border and `[x]`, so the two states
never depend on color alone.
Marks survive search, selected-only mode, and fullscreen navigation. Space sets
the anchor used by `v`; newly marked paths retain this interaction order.

Ctrl+s restores the terminal and writes the confirmed full paths to stdout.
`q` cancels with exit status 2 and no output. Escape in the grid is a no-op.
Confirming no marks is a successful empty output. Use `--print0` for arbitrary
Unix names, including names containing newlines or non-UTF-8 bytes:

```sh
red-table --select --print0 ./photos > selection.paths0
```

The terminal UI uses `/dev/tty`, never stdout, in this mode. For a producer-driven
collection, `--files0-from=-` reads NUL-delimited paths from stdin before opening
the interface. Relative input paths resolve from the current directory; duplicate
canonical paths appear once. Input is deliberately bounded to 1 MiB per record,
64 MiB total, and 1,000,000 records.

Selection mode deliberately skips the active escape-sequence capability probe
because the current renderer library hard-wires that probe to stdout. Guarded
environment detection and explicit `--graphics-protocol` selection remain
available; ordinary browsing retains full active negotiation.

## Shell integration

`red-table shell-init bash|zsh|fish` prints an embedded integration script. It
needs no terminal or configuration file and does not change your startup files.
The installed `red-table` executable must be available in `PATH`.

Add this line to `~/.bashrc` for Bash:

```bash
eval "$(red-table shell-init bash)"
```

For Zsh, initialize its completion system first, then load integration in
`~/.zshrc`. If your framework already calls `compinit`, put the second line after
that setup and omit the first line:

```zsh
autoload -Uz compinit && compinit
eval "$(red-table shell-init zsh)"
```

For Fish, add this to `~/.config/fish/config.fish`:

```fish
red-table shell-init fish | source
```

Open a new shell or run the matching setup line in the current one. Tab completes
options, quality levels, graphics protocols, thumbnail-size examples, shell names
and filesystem paths. Configuration paths use ordinary file completion. To browse
a directory literally named `shell-init`, use `red-table -- shell-init`.

Press `Alt+i` while composing a command, for example after `cp `, to select images
from the current directory and its subdirectories. Mark images with Space and
confirm with Ctrl+s. The widget inserts shell-quoted paths at the cursor, without
executing the command. `q`, an empty confirmation or an error leaves the command
line unchanged. Newlines, quotes, metacharacters and non-UTF-8 filename bytes are
preserved through NUL-delimited selection output. The widget declines to open
when the text before the cursor has unfinished syntax, such as an open quote or
command substitution. Invoke it between arguments outside quotes.

The binding is installed in Bash/Zsh emacs and vi insert maps, and Fish default
and insert maps. If Alt+i is already assigned, rebind `_red_table_pick` after the
initialization line, for example to Ctrl+x followed by i:

```bash
bind -x '"\C-xi":_red_table_pick'
```

```zsh
bindkey '^Xi' _red_table_pick
```

```fish
bind \cxi _red_table_pick
```

Removing the initialization line and starting a fresh shell removes integration.

## Yazi integration

The repository contains an installable `red-table.yazi` functional plugin for
Unix and Yazi 26.5.6 or newer. It opens Yazi's current real directory, gives
red-table temporary terminal ownership, and captures only the NUL-delimited
confirmed paths. Confirmation replaces Yazi's selection and reveals the first
result; confirmed empty output clears the selection. `q` cancellation and every
failure leave the existing selection unchanged.

After installing the plugin, add a key binding:

```toml
[[mgr.prepend_keymap]]
on   = [ "g", "i" ]
run  = "plugin red-table"
desc = "Select images with red-table"
```

The executable defaults to `red-table` in `PATH`. An absolute path, including a
Nix store path, can be configured in `~/.config/yazi/init.lua`:

```lua
require("red-table"):setup({
  command = "/absolute/path/to/red-table",
})
```

The plugin package README documents `ya pkg`, local, and Home Manager
installation. Virtual Yazi directories are intentionally rejected; enter a real
directory before invoking the plugin.

Every listed binding except the fixed `Ctrl+c` emergency interrupt can be
replaced in TOML. The concise status is the default; technical fields are shown
only while debug status is active.

Search is case-insensitive and matches both filenames and paths relative to the
opened root. Unsupported and corrupt images leave the loading state and show an
error tile with the decoder reason; they do not stop the browser.

`red-table` negotiates Kitty, Sixel, or iTerm2 graphics when the terminal exposes
them. These protocols retain the pixel resolution derived from the terminal's
font grid. Levels `1` through `9` trade preparation speed for Lanczos resampling
and progressively stronger edge enhancement; the current level appears as `Qn`
in the status line.

The protocol label also reports its source: `/auto` is an active negotiation,
`/env` is the guarded direct-Kitty recovery path, and `/forced` is an explicit
CLI override. Forced protocols can emit unsupported escape sequences when the
terminal does not implement them. Environment fallback is deliberately disabled
inside tmux; use an explicit override only after configuring passthrough there.
When Kitty is forced in tmux, red-table wraps graphics commands when a non-empty
`TMUX`, a `tmux*` `TERM`, or `TERM_PROGRAM=tmux` is present. This does not enable
tmux passthrough itself.

The Kitty backend transmits RGBA pixels into a virtual Unicode-placeholder
placement with an explicit row and column extent. It supports the protocol's
297 normative placeholder rows and reports larger terminal image areas as an
error instead of clipping them. Image data is marked transient, retransmitted
after a view is hidden and shown again, and explicitly deleted when its owned
cache object is discarded. If that path produces empty or
black areas in an otherwise unsupported terminal, restart with
`red-table --graphics-protocol halfblocks PATH` and report the original protocol
label. If Halfblocks also shows an error tile, the displayed decoder reason points
to the source file or format instead of the terminal renderer.

It falls back to colored Unicode half blocks otherwise. The status line labels
this renderer `Half 1x2/auto`: each terminal cell can represent only one
horizontal by two vertical color samples. This is a physical resolution limit,
not a source-image or cache failure. Quality levels change the resampling and
edge enhancement applied directly to that final sample grid, but cannot add more
samples. Use `+` for larger thumbnails or a terminal with Kitty, Sixel, or iTerm2
support when more visual detail is required.

When quality changes, an already prepared thumbnail remains visible only while
its new variant is pending. A completed failure replaces the fallback with an
error tile. The `load` counter reports replacements still in progress.

## Persistent thumbnails on Linux

Protocol-neutral PNG previews are shared through the freedesktop.org thumbnail
cache below `$XDG_CACHE_HOME/thumbnails`, or `$HOME/.cache/thumbnails` when the
XDG variable is unset. `D<hits>/<misses>` in the status line counts completed
persistent-cache outcomes for the current run. A warm hit avoids decoding the
original file; terminal- and quality-specific work still uses a memory cache
bounded to 256 entries and 128 MiB of accounted prepared data. Debug status
shows it as `M<entries>/<MiB>`. Thumbnail resize targets above 64 MiB of raw RGBA
data fail before allocation.

URI, file size, modification time, and red-table's processing version validate
entries. A changed or corrupt entry is regenerated automatically. The cache is
shared with other desktop applications: manually removing its size directories
is safe but also removes their regenerable thumbnails.
