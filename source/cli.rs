use std::{ffi::OsString, path::PathBuf};

use crate::{
    BoxError,
    config::{CliOverrides, ConfigSource},
    graphics::GraphicsProtocol,
    settings::{ThumbnailQuality, ThumbnailSize},
    shell::Shell,
};

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct BrowseOptions {
    pub(crate) path: PathBuf,
    pub(crate) config_source: ConfigSource,
    pub(crate) overrides: CliOverrides,
    pub(crate) select: bool,
    pub(crate) print0: bool,
    pub(crate) files0_from_stdin: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Command {
    Browse(BrowseOptions),
    Help,
    Version,
    ShellInit(Shell),
}

pub(crate) fn parse<I>(arguments: I) -> Result<Command, BoxError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut arguments = arguments.into_iter().peekable();
    if arguments
        .peek()
        .is_some_and(|argument| argument == "shell-init")
    {
        arguments.next();
        let shell = arguments
            .next()
            .ok_or("shell-init requires bash, zsh, or fish")?;
        if shell == "--help" || shell == "-h" {
            return Ok(Command::Help);
        }
        let shell = Shell::parse(&shell)?;
        if arguments.next().is_some() {
            return Err("shell-init accepts exactly one shell: bash, zsh, or fish".into());
        }
        return Ok(Command::ShellInit(shell));
    }
    let mut path = None;
    let mut overrides = CliOverrides::default();
    let mut config_source = ConfigSource::Discover;
    let mut select = false;
    let mut print0 = false;
    let mut files0_from_stdin = false;
    let mut options_ended = false;

    while let Some(argument) = arguments.next() {
        if argument == "-h" || argument == "--help" {
            return Ok(Command::Help);
        }
        if argument == "-V" || argument == "--version" {
            return Ok(Command::Version);
        }
        if !options_ended && argument == "--" {
            options_ended = true;
            continue;
        }
        if !options_ended && (argument == "-s" || argument == "--thumbnail-size") {
            let value = arguments
                .next()
                .ok_or("--thumbnail-size requires WIDTHxHEIGHT")?;
            overrides.thumbnail_size = Some(parse_thumbnail_size(&value)?);
            continue;
        }
        if !options_ended && argument == "--quality" {
            let value = arguments
                .next()
                .ok_or("--quality requires a level from 1 to 9")?;
            overrides.thumbnail_quality = Some(parse_quality(&value)?);
            continue;
        }
        if !options_ended && argument == "--graphics-protocol" {
            let value = arguments
                .next()
                .ok_or("--graphics-protocol requires auto, kitty, sixel, iterm2, or halfblocks")?;
            overrides.graphics_protocol = Some(parse_graphics_protocol(&value)?);
            continue;
        }
        if !options_ended && argument == "--config" {
            if config_source != ConfigSource::Discover {
                return Err("--config and --no-config may not be combined".into());
            }
            let value = arguments.next().ok_or("--config requires a path")?;
            if value.is_empty() {
                return Err("--config requires a non-empty path".into());
            }
            config_source = ConfigSource::Explicit(PathBuf::from(value));
            continue;
        }
        if !options_ended && argument == "--no-config" {
            if config_source != ConfigSource::Discover {
                return Err("--config and --no-config may not be combined".into());
            }
            config_source = ConfigSource::Disabled;
            continue;
        }
        if !options_ended && argument == "--select" {
            select = true;
            continue;
        }
        if !options_ended && argument == "--print0" {
            print0 = true;
            continue;
        }
        if !options_ended && argument == "--files0-from" {
            let value = arguments.next().ok_or("--files0-from requires -")?;
            if value != "-" {
                return Err("--files0-from currently accepts only - for standard input".into());
            }
            files0_from_stdin = true;
            continue;
        }
        if !options_ended
            && let Some(value) = argument
                .to_str()
                .and_then(|argument| argument.strip_prefix("--files0-from="))
        {
            if value != "-" {
                return Err("--files0-from currently accepts only - for standard input".into());
            }
            files0_from_stdin = true;
            continue;
        }
        if !options_ended
            && let Some(value) = argument
                .to_str()
                .and_then(|argument| argument.strip_prefix("--thumbnail-size="))
        {
            overrides.thumbnail_size = Some(
                value
                    .parse()
                    .map_err(|error: String| -> BoxError { error.into() })?,
            );
            continue;
        }
        if !options_ended
            && let Some(value) = argument
                .to_str()
                .and_then(|argument| argument.strip_prefix("--graphics-protocol="))
        {
            overrides.graphics_protocol = Some(
                value
                    .parse()
                    .map_err(|error: String| -> BoxError { error.into() })?,
            );
            continue;
        }
        if !options_ended
            && let Some(value) = argument
                .to_str()
                .and_then(|argument| argument.strip_prefix("--quality="))
        {
            overrides.thumbnail_quality = Some(
                value
                    .parse()
                    .map_err(|error: String| -> BoxError { error.into() })?,
            );
            continue;
        }
        if !options_ended
            && let Some(value) = argument
                .to_str()
                .and_then(|argument| argument.strip_prefix("--config="))
        {
            if config_source != ConfigSource::Discover {
                return Err("--config and --no-config may not be combined".into());
            }
            if value.is_empty() {
                return Err("--config requires a non-empty path".into());
            }
            config_source = ConfigSource::Explicit(PathBuf::from(value));
            continue;
        }
        if !options_ended && argument.to_string_lossy().starts_with('-') {
            return Err(format!("unknown option: {}", argument.to_string_lossy()).into());
        }
        if path.replace(PathBuf::from(&argument)).is_some() {
            return Err("only one path may be specified".into());
        }
    }

    if print0 && !select {
        return Err("--print0 requires --select".into());
    }
    if files0_from_stdin && path.is_some() {
        return Err("--files0-from may not be combined with a positional path".into());
    }

    Ok(Command::Browse(BrowseOptions {
        path: path.unwrap_or_else(|| PathBuf::from(".")),
        config_source,
        overrides,
        select,
        print0,
        files0_from_stdin,
    }))
}

fn parse_graphics_protocol(value: &OsString) -> Result<GraphicsProtocol, BoxError> {
    value
        .to_str()
        .ok_or_else(|| "graphics protocol must be valid UTF-8".into())
        .and_then(|value| value.parse().map_err(|error: String| error.into()))
}

fn parse_quality(value: &OsString) -> Result<ThumbnailQuality, BoxError> {
    value
        .to_str()
        .ok_or_else(|| "thumbnail quality must be valid UTF-8".into())
        .and_then(|value| value.parse().map_err(|error: String| error.into()))
}

fn parse_thumbnail_size(value: &OsString) -> Result<ThumbnailSize, BoxError> {
    value
        .to_str()
        .ok_or_else(|| "thumbnail size must be valid UTF-8".into())
        .and_then(|value| value.parse().map_err(|error: String| error.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_init_is_an_explicit_command_and_double_dash_keeps_its_path_available() {
        assert_eq!(
            parse(["shell-init", "bash"].map(OsString::from)).unwrap(),
            Command::ShellInit(Shell::Bash)
        );
        let Command::Browse(options) = parse(["--", "shell-init"].map(OsString::from)).unwrap()
        else {
            panic!("expected browsing")
        };
        assert_eq!(options.path, PathBuf::from("shell-init"));
    }

    #[test]
    fn defaults_to_current_directory() {
        assert_eq!(
            parse(Vec::new()).unwrap(),
            Command::Browse(BrowseOptions {
                path: ".".into(),
                config_source: ConfigSource::Discover,
                overrides: CliOverrides::default(),
                select: false,
                print0: false,
                files0_from_stdin: false,
            })
        );
    }

    #[test]
    fn rejects_unknown_options() {
        assert!(parse([OsString::from("--wat")]).is_err());
    }

    #[test]
    fn rejects_multiple_paths() {
        assert!(parse([OsString::from("one"), OsString::from("two")]).is_err());
    }

    #[test]
    fn accepts_all_thumbnail_size_forms() {
        for arguments in [
            vec!["-s", "40x18", "photos"],
            vec!["--thumbnail-size", "40x18", "photos"],
            vec!["--thumbnail-size=40x18", "photos"],
        ] {
            assert_eq!(
                parse(arguments.into_iter().map(OsString::from)).unwrap(),
                Command::Browse(BrowseOptions {
                    path: "photos".into(),
                    config_source: ConfigSource::Discover,
                    overrides: CliOverrides {
                        thumbnail_size: Some(ThumbnailSize {
                            width: 40,
                            height: 18
                        }),
                        ..CliOverrides::default()
                    },
                    select: false,
                    print0: false,
                    files0_from_stdin: false,
                })
            );
        }
    }

    #[test]
    fn double_dash_allows_a_path_starting_with_a_dash() {
        let command = parse([OsString::from("--"), OsString::from("-photos")]).unwrap();
        assert_eq!(
            command,
            Command::Browse(BrowseOptions {
                path: "-photos".into(),
                config_source: ConfigSource::Discover,
                overrides: CliOverrides::default(),
                select: false,
                print0: false,
                files0_from_stdin: false,
            })
        );
    }

    #[test]
    fn accepts_quality_in_separate_and_equals_forms() {
        for arguments in [
            vec!["--quality", "9", "photos"],
            vec!["--quality=9", "photos"],
        ] {
            assert_eq!(
                parse(arguments.into_iter().map(OsString::from)).unwrap(),
                Command::Browse(BrowseOptions {
                    path: "photos".into(),
                    config_source: ConfigSource::Discover,
                    overrides: CliOverrides {
                        thumbnail_quality: Some("9".parse().unwrap()),
                        ..CliOverrides::default()
                    },
                    select: false,
                    print0: false,
                    files0_from_stdin: false,
                })
            );
        }
    }

    #[test]
    fn accepts_graphics_protocol_in_separate_and_equals_forms() {
        for arguments in [
            vec!["--graphics-protocol", "kitty", "photos"],
            vec!["--graphics-protocol=kitty", "photos"],
        ] {
            assert_eq!(
                parse(arguments.into_iter().map(OsString::from)).unwrap(),
                Command::Browse(BrowseOptions {
                    path: "photos".into(),
                    config_source: ConfigSource::Discover,
                    overrides: CliOverrides {
                        graphics_protocol: Some(GraphicsProtocol::Kitty),
                        ..CliOverrides::default()
                    },
                    select: false,
                    print0: false,
                    files0_from_stdin: false,
                })
            );
        }
    }

    #[test]
    fn parses_config_sources_and_rejects_conflicts() {
        assert_eq!(
            parse([OsString::from("--config=custom.toml")]).unwrap(),
            Command::Browse(BrowseOptions {
                path: ".".into(),
                config_source: ConfigSource::Explicit("custom.toml".into()),
                overrides: CliOverrides::default(),
                select: false,
                print0: false,
                files0_from_stdin: false,
            })
        );
        assert_eq!(
            parse([OsString::from("--no-config")]).unwrap(),
            Command::Browse(BrowseOptions {
                path: ".".into(),
                config_source: ConfigSource::Disabled,
                overrides: CliOverrides::default(),
                select: false,
                print0: false,
                files0_from_stdin: false,
            })
        );
        assert!(
            parse([
                OsString::from("--config"),
                OsString::from("one.toml"),
                OsString::from("--no-config")
            ])
            .is_err()
        );
    }

    #[test]
    fn parses_selection_and_stdin_modes_with_conflict_checks() {
        assert_eq!(
            parse([
                OsString::from("--select"),
                OsString::from("--print0"),
                OsString::from("--files0-from=-")
            ])
            .unwrap(),
            Command::Browse(BrowseOptions {
                path: ".".into(),
                config_source: ConfigSource::Discover,
                overrides: CliOverrides::default(),
                select: true,
                print0: true,
                files0_from_stdin: true,
            })
        );
        assert!(parse([OsString::from("--print0")]).is_err());
        assert!(
            parse([
                OsString::from("--files0-from"),
                OsString::from("-"),
                OsString::from("photos")
            ])
            .is_err()
        );
        assert!(
            parse([
                OsString::from("--files0-from"),
                OsString::from("list.paths0")
            ])
            .is_err()
        );
    }
}
