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

Directory scans show images in byte-wise ascending order of their path relative
to the scanned directory, the order `ls` uses. Images found later in a running
scan are merged into that order. A `--files0-from` list keeps its input order.

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
