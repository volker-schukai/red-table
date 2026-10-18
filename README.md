# red-table

`red-table` is a performance-first terminal image browser for large image
collections. It provides a searchable, scrollable thumbnail grid controlled
entirely from the keyboard, including arrow keys and Vim-style `h`, `j`, `k`,
and `l` navigation.

The application is implemented in Rust and distributed with reproducible Nix
build metadata.

## Showcase

[Watch the 19-second terminal demo on YouTube](https://youtu.be/rcg16ZPUYgA).

The real Kitty session starts at a neutral terminal prompt, launches `red-table`,
opens the sharp eight-image grid, marks and inspects an image fullscreen, returns
to the exact grid position, and finishes in the two-pane A/B comparison. The
eight neutral sample images shown in the recording are included in
[`media/showcase`](media/showcase/) so the workflow can be tried locally.

## Development environment

Nix and direnv are the only bootstrap requirements. All project tools,
including Rust, Cargo, Task, Git, formatters, linters, and MkDocs, are supplied
by the locked flake.

```sh
direnv allow
task check
```

Without direnv, enter exactly the same environment explicitly:

```sh
nix develop
task check
```

Build and run the reproducible package with:

```sh
nix build
nix run . -- /path/to/photos
```

Choose a preferred thumbnail cell size at startup:

```sh
nix run . -- --thumbnail-size 40x18 /path/to/photos
```

Select a startup quality from `1` (fast) to `9` (maximum detail enhancement):

```sh
nix run . -- --quality 9 /path/to/photos
```

Terminal graphics are detected automatically. For diagnosis or recovery, select
a protocol explicitly:

```sh
nix run . -- --graphics-protocol kitty /path/to/photos
```

User-facing defaults and key bindings can be configured through strict TOML at
`$XDG_CONFIG_HOME/red-table/config.toml`. Use `--config PATH` to require another
file or `--no-config` for a reproducible built-in-only start. See the
[configuration reference](documentation/references/configuration.md).

Run `task` to list every development command.

## Controls

- arrow keys or `h`, `j`, `k`, `l`: move the selection
- Enter: inspect the focused image fullscreen; Escape returns to the exact grid position
- `c`: compare the focused reference against candidates in two panes
- `s`: toggle discovery / path order; sessions start in stable discovery order
- Page Up, Page Down, Home, End: move across larger collections
- `+` / `-`: enlarge or shrink thumbnails; `0` restores the `32x14` default
- `1` through `9`: select thumbnail quality immediately; the default is `7`
- `/`: search filenames and relative paths; Enter applies and Escape cancels
- `?`: show help generated from the active key bindings
- Space: mark or unmark the focused image; marks show `[x]` and a double border
- `v`: mark the inclusive range from the last Space anchor
- `u` / `m`: clear marks / show only marked images
- Ctrl+s: confirm paths in `--select` mode
- `F12`: toggle renderer, quality, queue, and cache diagnostics
- `q`: quit; Escape only closes or cancels the active nested view

In fullscreen inspection, `p` / `n` select the previous / next match, `z`
switches between fit and exact 100%, and `+` / `-` select the bounded 25–800%
zoom levels. Arrows or `h`, `j`, `k`, `l` pan the crop; `b` cycles the
checkerboard, dark, and light transparency backgrounds. Decoding and crop
preparation remain off the input thread, and EXIF orientation is applied before
display.

In A/B comparison, the left `REFERENCE` stays pinned while `p` / `n` changes the
right `CANDIDATE`. Enter promotes the candidate and continues with another image.
Tab chooses the active pane; Space marks that pane's image. `s` switches between
coupled and independent zoom/pan. When coupled, normalized centers and zoom levels
stay identical even for different source dimensions. Escape returns to the grid.

Use red-table as a visual shell filter with:

```sh
nix run . -- --select --print0 /path/to/photos > selection.paths0
```

The interface is written only to `/dev/tty`; confirmed full paths are the only
bytes written to stdout. NUL mode preserves arbitrary Unix filename bytes and is
safe for tools such as `xargs -0`. Without `--print0`, one full path is emitted
per line for human-readable workflows. `q` cancels with status 2 and no output;
Ctrl+s confirms, including a successful empty selection. A producer
can provide an ordered NUL-delimited file list instead of a directory:

```sh
find ./photos -type f -print0 |
  nix run . -- --select --print0 --files0-from=- > selection.paths0
```

To keep stdout pristine, `--select` skips the active escape-sequence capability
probe. Guarded terminal environment hints still select direct Kitty/iTerm2, and
`--graphics-protocol` remains available when an explicit renderer is required.

## Yazi plugin

The installable [`red-table.yazi`](red-table.yazi/) functional plugin opens the
current real Yazi directory in `red-table --select --print0`, temporarily hands
red-table the terminal, and replaces Yazi's selection only after successful
confirmation. Status-2 cancellation leaves Yazi unchanged. The bridge requires
Unix and Yazi 26.5.6 or newer; `red-table` must be in `PATH` or configured as
an absolute executable in Yazi's `init.lua`.

Bind it after installing the plugin:

```toml
[[mgr.prepend_keymap]]
on   = [ "g", "i" ]
run  = "plugin red-table"
desc = "Select images with red-table"
```

See the [plugin README](red-table.yazi/README.md) for `ya pkg`, local checkout,
Home Manager, and executable configuration instructions. The flake exposes the
plugin separately as `packages.yazi-plugin`.

JPEG, PNG, GIF, WebP, TIFF, and BMP files are discovered recursively. Startup
sizes between `12x6` and `120x60` terminal cells are accepted.

Quality levels trade preparation latency for edge definition: `1` is the
fastest, `3` is the former Catmull-Rom policy, and `4` through `9` use Lanczos3
with progressively stronger bounded detail enhancement. For the highest visible
detail, use a terminal exposing Kitty, Sixel, or iTerm2 graphics. The portable
`Half 1x2` fallback has only one horizontal and two vertical color samples per
terminal cell; increase the thumbnail size with `+` when that fallback is active.
Within that limit, Q1–Q9 now control the final Halfblocks sampling and sharpening
rather than an intermediate image.
The renderer status includes its selection source, for example `Kitty/auto`,
`Kitty/env`, `Kitty/forced`, or `Half 1x2/auto`. A forced protocol must be
supported by the terminal.

Kitty rendering uses a complete Unicode-placeholder virtual placement with an
explicit cell extent. Retained images are retransmitted after leaving and
re-entering a view, and discarded image IDs are removed from terminal storage.
Inside tmux, commands are wrapped when `TMUX`, `TERM`, or `TERM_PROGRAM`
identifies the multiplexer; tmux passthrough must still be configured externally.
If a terminal still shows empty or black image areas, use
`red-table --graphics-protocol halfblocks PATH` as a safe recovery path and
include the status label when reporting the compatibility issue. Decode failures
leave `loading…` and appear as bounded error tiles with their reason.

## Performance architecture

The application keeps scanning, thumbnail production, search, input, and
rendering independent. It scans incrementally, decodes only the visible viewport
and one prefetch row on bounded background workers, keeps at most 256 prepared
thumbnails and 128 MiB of accounted protocol data in memory, and redraws only
after actual state changes. Debug status reports this cache as
`M<entries>/<MiB>`.

Fullscreen inspection and both comparison panes use separate four-request,
generation-cancelled workers but share one byte-bounded decoded-source cache
(512 MiB by default). Rapid navigation, zoom, and pan therefore discard obsolete
work without filling an unbounded queue or multiplying the decoded-memory limit.

On Linux, protocol-neutral previews persist in the shared freedesktop.org cache
below `$XDG_CACHE_HOME/thumbnails` (or `$HOME/.cache/thumbnails`). The status
field `D<hits>/<misses>` shows completed disk hits and misses. Source URI, byte
size, second- and nanosecond-resolution modification time, and the red-table
processing schema protect cache validity. Terminal protocol, geometry, and Q1–Q9
variants remain in the bounded RAM cache. See the
[cache reference](documentation/references/thumbnail-cache.md) for the complete
contract and cleanup implications.

In a true terminal application, the terminal emulator owns GPU rendering.
`red-table` will therefore target efficient terminal image protocols such as
Kitty and Sixel first. Measurements will determine whether a later optional
native `wgpu` frontend is warranted; the core will remain frontend-independent.

## Documentation

Build the local documentation site with `task docs-build`, or serve it with
`task docs-serve`.

## License

`red-table` is licensed under MIT. The crate is marked `publish = false`; binary
distribution does not publish its source through crates.io.
