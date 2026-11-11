use std::{collections::BTreeMap, env, fs, path::PathBuf};

use serde::Deserialize;

use crate::{
    BoxError,
    actions::{Action, ActionContext, Bindings, KeyChord},
    graphics::GraphicsProtocol,
    settings::{
        BackgroundMode, DEFAULT_THUMBNAIL_QUALITY, DEFAULT_THUMBNAIL_SIZE, ThumbnailQuality,
        ThumbnailSize,
    },
};

const CONFIG_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ConfigSource {
    Discover,
    Explicit(PathBuf),
    Disabled,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CliOverrides {
    pub(crate) thumbnail_size: Option<ThumbnailSize>,
    pub(crate) thumbnail_quality: Option<ThumbnailQuality>,
    pub(crate) graphics_protocol: Option<GraphicsProtocol>,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeSettings {
    pub(crate) thumbnail_size: ThumbnailSize,
    pub(crate) thumbnail_quality: ThumbnailQuality,
    pub(crate) graphics_protocol: GraphicsProtocol,
    pub(crate) debug_status: bool,
    pub(crate) background: BackgroundMode,
    pub(crate) decoded_cache_bytes: u64,
    pub(crate) filmstrip: bool,
    pub(crate) bindings: Bindings,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u16,
    thumbnail: Option<ThumbnailDocument>,
    graphics: Option<GraphicsDocument>,
    ui: Option<UiDocument>,
    keys: Option<KeysDocument>,
    inspection: Option<InspectionDocument>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThumbnailDocument {
    size: Option<String>,
    quality: Option<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphicsDocument {
    protocol: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UiDocument {
    debug_status: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeysDocument {
    normal: Option<BTreeMap<String, Vec<String>>>,
    search: Option<BTreeMap<String, Vec<String>>>,
    inspect: Option<BTreeMap<String, Vec<String>>>,
    compare: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectionDocument {
    background: Option<String>,
    decoded_cache_mib: Option<u64>,
    filmstrip: Option<bool>,
}

impl RuntimeSettings {
    pub(crate) fn load(source: &ConfigSource, cli: &CliOverrides) -> Result<Self, BoxError> {
        let document = load_document(source)?;
        let mut settings = Self {
            thumbnail_size: DEFAULT_THUMBNAIL_SIZE,
            thumbnail_quality: DEFAULT_THUMBNAIL_QUALITY,
            graphics_protocol: GraphicsProtocol::Auto,
            debug_status: false,
            background: BackgroundMode::Checkerboard,
            decoded_cache_bytes: 512 * 1024 * 1024,
            filmstrip: true,
            bindings: Bindings::built_in(),
        };
        if let Some(document) = document {
            settings.apply(document)?;
        }
        if let Some(value) = cli.thumbnail_size {
            settings.thumbnail_size = value;
        }
        if let Some(value) = cli.thumbnail_quality {
            settings.thumbnail_quality = value;
        }
        if let Some(value) = cli.graphics_protocol {
            settings.graphics_protocol = value;
        }
        Ok(settings)
    }

    fn apply(&mut self, document: Document) -> Result<(), BoxError> {
        if document.version != CONFIG_VERSION {
            return Err(format!(
                "unsupported configuration version {}; expected {CONFIG_VERSION}",
                document.version
            )
            .into());
        }
        if let Some(thumbnail) = document.thumbnail {
            if let Some(size) = thumbnail.size {
                self.thumbnail_size = size
                    .parse()
                    .map_err(|error: String| format!("thumbnail.size: {error}"))?;
            }
            if let Some(quality) = thumbnail.quality {
                self.thumbnail_quality = quality
                    .to_string()
                    .parse()
                    .map_err(|error: String| format!("thumbnail.quality: {error}"))?;
            }
        }
        if let Some(graphics) = document.graphics
            && let Some(protocol) = graphics.protocol
        {
            self.graphics_protocol = protocol
                .parse()
                .map_err(|error: String| format!("graphics.protocol: {error}"))?;
        }
        if let Some(ui) = document.ui
            && let Some(debug_status) = ui.debug_status
        {
            self.debug_status = debug_status;
        }
        if let Some(keys) = document.keys {
            self.apply_keys(ActionContext::Normal, keys.normal)?;
            self.apply_keys(ActionContext::Search, keys.search)?;
            self.apply_keys(ActionContext::Inspect, keys.inspect)?;
            self.apply_keys(ActionContext::Compare, keys.compare)?;
        }
        if let Some(inspection) = document.inspection {
            if let Some(background) = inspection.background {
                self.background = background
                    .parse()
                    .map_err(|error: String| format!("inspection.background: {error}"))?;
            }
            if let Some(mebibytes) = inspection.decoded_cache_mib {
                if !(64..=2048).contains(&mebibytes) {
                    return Err("inspection.decoded_cache_mib must be from 64 to 2048".into());
                }
                self.decoded_cache_bytes = mebibytes * 1024 * 1024;
            }
            if let Some(filmstrip) = inspection.filmstrip {
                self.filmstrip = filmstrip;
            }
        }
        Ok(())
    }

    fn apply_keys(
        &mut self,
        context: ActionContext,
        actions: Option<BTreeMap<String, Vec<String>>>,
    ) -> Result<(), BoxError> {
        let context_name = context_name(context);
        let mut configured = Vec::new();
        for (name, keys) in actions.unwrap_or_default() {
            let action = Action::from_name(&name)
                .map_err(|error| format!("keys.{context_name}: {error}"))?;
            let keys = keys
                .iter()
                .map(|key| key.parse::<KeyChord>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("keys.{context_name}.{name}: {error}"))?;
            configured.push((action, keys));
        }
        self.bindings
            .configure_context(context, configured)
            .map_err(|error| format!("keys.{context_name}: {error}").into())
    }
}

fn context_name(context: ActionContext) -> &'static str {
    match context {
        ActionContext::Normal => "normal",
        ActionContext::Search => "search",
        ActionContext::Inspect => "inspect",
        ActionContext::Compare => "compare",
    }
}

fn load_document(source: &ConfigSource) -> Result<Option<Document>, BoxError> {
    let (path, required) = match source {
        ConfigSource::Disabled => return Ok(None),
        ConfigSource::Explicit(path) => (Some(path.clone()), true),
        ConfigSource::Discover => (discovered_path(), false),
    };
    let Some(path) = path else {
        return Ok(None);
    };
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!("cannot read configuration {}: {error}", path.display()).into());
        }
    };
    toml::from_str(&contents)
        .map(Some)
        .map_err(|error| format!("invalid configuration {}: {error}", path.display()).into())
}

fn discovered_path() -> Option<PathBuf> {
    if let Some(root) = env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(root).join("red-table/config.toml"));
    }
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|root| root.join(".config/red-table/config.toml"))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temporary_file(name: &str, contents: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "red-table-config-{name}-{}-{nonce}.toml",
            std::process::id()
        ));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn missing_discovered_configuration_uses_defaults() {
        let settings =
            RuntimeSettings::load(&ConfigSource::Disabled, &CliOverrides::default()).unwrap();
        assert_eq!(settings.thumbnail_size, DEFAULT_THUMBNAIL_SIZE);
        assert_eq!(settings.thumbnail_quality, DEFAULT_THUMBNAIL_QUALITY);
        assert_eq!(settings.graphics_protocol, GraphicsProtocol::Auto);
    }

    #[test]
    fn cli_overrides_loaded_configuration() {
        let path = temporary_file(
            "precedence",
            r#"
version = 1
[thumbnail]
size = "40x18"
quality = 2
[graphics]
protocol = "halfblocks"
"#,
        );
        let settings = RuntimeSettings::load(
            &ConfigSource::Explicit(path.clone()),
            &CliOverrides {
                thumbnail_quality: Some("9".parse().unwrap()),
                ..CliOverrides::default()
            },
        )
        .unwrap();
        assert_eq!(settings.thumbnail_size.to_string(), "40x18");
        assert_eq!(settings.thumbnail_quality.to_string(), "9");
        assert_eq!(settings.graphics_protocol, GraphicsProtocol::Halfblocks);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn loads_inspection_memory_background_and_bindings() {
        let path = temporary_file(
            "inspection",
            r#"
version = 1
[inspection]
background = "dark"
decoded_cache_mib = 256
filmstrip = false
[keys.inspect]
fit_100 = ["f"]
[keys.compare]
promote = ["P"]
"#,
        );
        let settings = RuntimeSettings::load(
            &ConfigSource::Explicit(path.clone()),
            &CliOverrides::default(),
        )
        .unwrap();
        assert_eq!(settings.background, BackgroundMode::Dark);
        assert_eq!(settings.decoded_cache_bytes, 256 * 1024 * 1024);
        assert!(!settings.filmstrip);
        assert_eq!(
            settings.bindings.action(
                ActionContext::Inspect,
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('f'),
                    crossterm::event::KeyModifiers::NONE,
                ),
            ),
            Some(Action::ToggleFitHundred)
        );
        assert_eq!(
            settings.bindings.action(
                ActionContext::Compare,
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('p'),
                    crossterm::event::KeyModifiers::SHIFT,
                ),
            ),
            Some(Action::PromoteCandidate)
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_unknown_fields_versions_actions_and_conflicts() {
        for (name, contents) in [
            ("field", "version = 1\nwat = true\n"),
            ("version", "version = 2\n"),
            ("action", "version = 1\n[keys.normal]\nteleport = [\"t\"]\n"),
            (
                "conflict",
                "version = 1\n[keys.normal]\nleft = [\"x\"]\nright = [\"x\"]\n",
            ),
            (
                "inspection-memory",
                "version = 1\n[inspection]\ndecoded_cache_mib = 8\n",
            ),
        ] {
            let path = temporary_file(name, contents);
            assert!(
                RuntimeSettings::load(
                    &ConfigSource::Explicit(path.clone()),
                    &CliOverrides::default()
                )
                .is_err(),
                "{name} must fail"
            );
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn user_keys_displace_built_in_bindings_and_allow_swaps() {
        let path = temporary_file(
            "precedence",
            r#"
version = 1
[keys.normal]
search = ["s"]
toggle_sort = ["/"]
[keys.inspect]
pan_left = ["Left", "h"]
"#,
        );
        let settings = RuntimeSettings::load(
            &ConfigSource::Explicit(path.clone()),
            &CliOverrides::default(),
        )
        .unwrap();
        let press = |context, code| {
            settings.bindings.action(
                context,
                crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE),
            )
        };
        use crossterm::event::KeyCode;
        assert_eq!(
            press(ActionContext::Normal, KeyCode::Char('s')),
            Some(Action::StartSearch)
        );
        assert_eq!(
            press(ActionContext::Normal, KeyCode::Char('/')),
            Some(Action::ToggleSort)
        );
        assert_eq!(
            press(ActionContext::Inspect, KeyCode::Left),
            Some(Action::PanLeft)
        );
        assert_eq!(
            press(ActionContext::Inspect, KeyCode::Right),
            Some(Action::NextOrPanRight)
        );
        assert_eq!(
            press(ActionContext::Normal, KeyCode::Left),
            Some(Action::MoveLeft),
            "other contexts keep their defaults"
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn missing_explicit_configuration_is_an_error() {
        let path = env::temp_dir().join(format!("red-table-missing-config-{}", std::process::id()));
        let error = RuntimeSettings::load(
            &ConfigSource::Explicit(path.clone()),
            &CliOverrides::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains(&path.display().to_string()));
    }
}
