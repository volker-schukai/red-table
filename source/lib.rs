mod actions;
mod app;
mod browser;
mod cli;
mod config;
pub mod diagnostics;
mod graphics;
mod image_source;
mod inspection;
mod kitty;
mod path_io;
mod persistent_cache;
mod refresh;
mod scan;
mod settings;
mod shell;
mod terminal;
mod thumbnails;
mod ui;

use std::{error::Error, ffi::OsString, io};

pub type BoxError = Box<dyn Error + Send + Sync>;

pub const HELP: &str = "red-table - a performance-first terminal image browser

Usage: red-table [OPTIONS] [PATH]
       red-table shell-init <bash|zsh|fish>

Commands:
  shell-init SHELL  Print completion and Alt+i image-picker integration
                    (use -- shell-init to browse a directory named shell-init)

Arguments:
  [PATH]           Directory to browse or image to inspect (default: current directory)

Options:
  -s, --thumbnail-size WIDTHxHEIGHT
                   Target thumbnail size in terminal cells (default: 32x14)
      --quality LEVEL
                   Thumbnail quality from 1 (fast) to 9 (maximum; default: 7)
      --graphics-protocol PROTOCOL
                   auto, kitty, sixel, iterm2, or halfblocks (default: auto)
      --config PATH Load this TOML configuration instead of XDG discovery
      --no-config   Disable configuration discovery
      --select      Confirm marked paths to stdout; terminal UI uses /dev/tty
      --print0      NUL-delimit selected paths (requires --select)
      --files0-from -
                   Read a NUL-delimited input path list from stdin
  -h, --help       Print help
  -V, --version    Print version

Controls:
  arrows, hjkl     Move selection
  Enter            Inspect focused image fullscreen
  c                Compare focused image against candidates
  +, -, 0          Enlarge, shrink, or reset thumbnails
  1..9             Set thumbnail quality
  /                Search by filename or path
  ?                Show contextual key help
  Space, v, u, m   Toggle mark, mark range, clear, or show marked only
  Ctrl+s           Confirm paths in --select mode
  q                Quit

Inspection:
  Esc              Return to the exact grid position
  p, n             Previous or next filtered image
  z, +, -          Fit/100%, zoom in, or zoom out
  arrows, hjkl     Pan the image crop
  b                Cycle transparency background
  Space            Toggle the current mark

Comparison:
  p, n             Previous or next candidate
  Enter            Promote candidate to reference
  Tab, s           Switch active pane or synchronize views
  z, +, -, hjkl    Fit/100%, zoom, and pan active view
  Space            Toggle mark on active pane
";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunOutcome {
    Completed,
    Cancelled,
}

impl RunOutcome {
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Completed => 0,
            Self::Cancelled => 2,
        }
    }
}

pub fn run_from<I>(arguments: I) -> Result<RunOutcome, BoxError>
where
    I: IntoIterator<Item = OsString>,
{
    match cli::parse(arguments)? {
        cli::Command::ShellInit(shell) => {
            use io::Write;
            io::stdout().lock().write_all(shell.script().as_bytes())?;
            Ok(RunOutcome::Completed)
        }
        cli::Command::Help => {
            print!("{HELP}");
            Ok(RunOutcome::Completed)
        }
        cli::Command::Version => {
            println!("red-table {}", env!("CARGO_PKG_VERSION"));
            Ok(RunOutcome::Completed)
        }
        cli::Command::Browse(options) => {
            let settings =
                config::RuntimeSettings::load(&options.config_source, &options.overrides)?;
            let input_paths = options
                .files0_from_stdin
                .then(|| {
                    path_io::read_nul_paths(io::stdin().lock())
                        .map_err(|error| format!("cannot read --files0-from stdin: {error}"))
                })
                .transpose()?;
            match browser::run(options.path, settings, options.select, input_paths)? {
                browser::BrowseOutcome::Completed => Ok(RunOutcome::Completed),
                browser::BrowseOutcome::Cancelled => Ok(RunOutcome::Cancelled),
                browser::BrowseOutcome::Selection(paths) => {
                    path_io::write_paths(&mut io::stdout().lock(), &paths, options.print0)?;
                    Ok(RunOutcome::Completed)
                }
            }
        }
    }
}
