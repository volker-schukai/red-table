use std::{collections::HashMap, fmt, str::FromStr};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ActionContext {
    Normal,
    Search,
    Inspect,
    Compare,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Action {
    Quit,
    StartSearch,
    ToggleSort,
    CancelSearch,
    CommitSearch,
    DeleteSearchCharacter,
    ThumbnailLarger,
    ThumbnailSmaller,
    ThumbnailReset,
    SetQuality(u8),
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    PageUp,
    PageDown,
    First,
    Last,
    ToggleHelp,
    ToggleDebug,
    OpenInspect,
    CloseInspect,
    PreviousImage,
    NextImage,
    ToggleFitHundred,
    ZoomIn,
    ZoomOut,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
    CycleBackground,
    ToggleMark,
    MarkRange,
    ClearMarks,
    ToggleMarkedOnly,
    ConfirmSelection,
    OpenCompare,
    PromoteCandidate,
    SwitchComparePane,
    ToggleCompareSync,
}

impl Action {
    pub(crate) fn from_name(name: &str) -> Result<Self, String> {
        match name {
            "quit" => Ok(Self::Quit),
            "search" => Ok(Self::StartSearch),
            "toggle_sort" => Ok(Self::ToggleSort),
            "cancel" => Ok(Self::CancelSearch),
            "confirm" => Ok(Self::CommitSearch),
            "backspace" => Ok(Self::DeleteSearchCharacter),
            "thumbnail_larger" => Ok(Self::ThumbnailLarger),
            "thumbnail_smaller" => Ok(Self::ThumbnailSmaller),
            "thumbnail_reset" => Ok(Self::ThumbnailReset),
            "quality_1" => Ok(Self::SetQuality(1)),
            "quality_2" => Ok(Self::SetQuality(2)),
            "quality_3" => Ok(Self::SetQuality(3)),
            "quality_4" => Ok(Self::SetQuality(4)),
            "quality_5" => Ok(Self::SetQuality(5)),
            "quality_6" => Ok(Self::SetQuality(6)),
            "quality_7" => Ok(Self::SetQuality(7)),
            "quality_8" => Ok(Self::SetQuality(8)),
            "quality_9" => Ok(Self::SetQuality(9)),
            "left" => Ok(Self::MoveLeft),
            "right" => Ok(Self::MoveRight),
            "up" => Ok(Self::MoveUp),
            "down" => Ok(Self::MoveDown),
            "page_up" => Ok(Self::PageUp),
            "page_down" => Ok(Self::PageDown),
            "first" => Ok(Self::First),
            "last" => Ok(Self::Last),
            "help" => Ok(Self::ToggleHelp),
            "debug" => Ok(Self::ToggleDebug),
            "inspect" => Ok(Self::OpenInspect),
            "close" => Ok(Self::CloseInspect),
            "previous" => Ok(Self::PreviousImage),
            "next" => Ok(Self::NextImage),
            "fit_100" => Ok(Self::ToggleFitHundred),
            "zoom_in" => Ok(Self::ZoomIn),
            "zoom_out" => Ok(Self::ZoomOut),
            "pan_left" => Ok(Self::PanLeft),
            "pan_right" => Ok(Self::PanRight),
            "pan_up" => Ok(Self::PanUp),
            "pan_down" => Ok(Self::PanDown),
            "background" => Ok(Self::CycleBackground),
            "toggle_mark" => Ok(Self::ToggleMark),
            "mark_range" => Ok(Self::MarkRange),
            "clear_marks" => Ok(Self::ClearMarks),
            "marked_only" => Ok(Self::ToggleMarkedOnly),
            "confirm_selection" => Ok(Self::ConfirmSelection),
            "compare" => Ok(Self::OpenCompare),
            "promote" => Ok(Self::PromoteCandidate),
            "switch_pane" => Ok(Self::SwitchComparePane),
            "sync" => Ok(Self::ToggleCompareSync),
            _ => Err(format!("unknown action `{name}`")),
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::StartSearch => "search",
            Self::ToggleSort => "discovery / path order",
            Self::CancelSearch => "cancel",
            Self::CommitSearch => "confirm",
            Self::DeleteSearchCharacter => "delete character",
            Self::ThumbnailLarger => "larger thumbnails",
            Self::ThumbnailSmaller => "smaller thumbnails",
            Self::ThumbnailReset => "reset thumbnail size",
            Self::SetQuality(1) => "quality 1",
            Self::SetQuality(2) => "quality 2",
            Self::SetQuality(3) => "quality 3",
            Self::SetQuality(4) => "quality 4",
            Self::SetQuality(5) => "quality 5",
            Self::SetQuality(6) => "quality 6",
            Self::SetQuality(7) => "quality 7",
            Self::SetQuality(8) => "quality 8",
            Self::SetQuality(9) => "quality 9",
            Self::SetQuality(_) => "quality",
            Self::MoveLeft => "move left",
            Self::MoveRight => "move right",
            Self::MoveUp => "move up",
            Self::MoveDown => "move down",
            Self::PageUp => "page up",
            Self::PageDown => "page down",
            Self::First => "first image",
            Self::Last => "last image",
            Self::ToggleHelp => "help",
            Self::ToggleDebug => "debug status",
            Self::OpenInspect => "inspect image",
            Self::CloseInspect => "return to grid",
            Self::PreviousImage => "previous image",
            Self::NextImage => "next image",
            Self::ToggleFitHundred => "fit / 100%",
            Self::ZoomIn => "zoom in",
            Self::ZoomOut => "zoom out",
            Self::PanLeft => "pan left",
            Self::PanRight => "pan right",
            Self::PanUp => "pan up",
            Self::PanDown => "pan down",
            Self::CycleBackground => "transparency background",
            Self::ToggleMark => "mark / unmark image",
            Self::MarkRange => "mark anchored range",
            Self::ClearMarks => "clear all marks",
            Self::ToggleMarkedOnly => "show only marked",
            Self::ConfirmSelection => "confirm selection",
            Self::OpenCompare => "compare with candidates",
            Self::PromoteCandidate => "promote candidate",
            Self::SwitchComparePane => "switch active pane",
            Self::ToggleCompareSync => "synchronize views",
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct KeyChord {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyChord {
    fn from_event(event: KeyEvent) -> Self {
        let mut code = event.code;
        let mut modifiers =
            event.modifiers & (KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT);
        if let KeyCode::Char(character) = code
            && character.is_ascii_uppercase()
        {
            code = KeyCode::Char(character.to_ascii_lowercase());
            modifiers.insert(KeyModifiers::SHIFT);
        }
        if matches!(code, KeyCode::Char(character) if !character.is_ascii_alphanumeric()) {
            modifiers.remove(KeyModifiers::SHIFT);
        }
        Self { code, modifiers }
    }

    fn is_emergency_interrupt(&self) -> bool {
        self.code == KeyCode::Char('c') && self.modifiers == KeyModifiers::CONTROL
    }
}

impl FromStr for KeyChord {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.chars().count() == 1 {
            let mut character = value.chars().next().expect("count is one");
            let mut modifiers = KeyModifiers::NONE;
            if character.is_ascii_uppercase() {
                character = character.to_ascii_lowercase();
                modifiers.insert(KeyModifiers::SHIFT);
            }
            return Ok(Self {
                code: KeyCode::Char(character),
                modifiers,
            });
        }
        let mut parts = value.split('+').collect::<Vec<_>>();
        let code_name = parts.pop().ok_or("key binding cannot be empty")?;
        if code_name.is_empty() {
            return Err(format!("invalid key binding `{value}`"));
        }
        let mut modifiers = KeyModifiers::NONE;
        for modifier in parts {
            let flag = match modifier {
                "Ctrl" => KeyModifiers::CONTROL,
                "Alt" => KeyModifiers::ALT,
                "Shift" => KeyModifiers::SHIFT,
                _ => return Err(format!("unknown key modifier `{modifier}` in `{value}`")),
            };
            if modifiers.contains(flag) {
                return Err(format!("duplicate key modifier `{modifier}` in `{value}`"));
            }
            modifiers.insert(flag);
        }

        let code = match code_name {
            "Enter" => KeyCode::Enter,
            "Esc" => KeyCode::Esc,
            "Space" => KeyCode::Char(' '),
            "Backspace" => KeyCode::Backspace,
            "Left" => KeyCode::Left,
            "Right" => KeyCode::Right,
            "Up" => KeyCode::Up,
            "Down" => KeyCode::Down,
            "PageUp" => KeyCode::PageUp,
            "PageDown" => KeyCode::PageDown,
            "Home" => KeyCode::Home,
            "End" => KeyCode::End,
            "Tab" => KeyCode::Tab,
            "F1" => KeyCode::F(1),
            "F2" => KeyCode::F(2),
            "F3" => KeyCode::F(3),
            "F4" => KeyCode::F(4),
            "F5" => KeyCode::F(5),
            "F6" => KeyCode::F(6),
            "F7" => KeyCode::F(7),
            "F8" => KeyCode::F(8),
            "F9" => KeyCode::F(9),
            "F10" => KeyCode::F(10),
            "F11" => KeyCode::F(11),
            "F12" => KeyCode::F(12),
            _ if code_name.chars().count() == 1 => {
                let mut character = code_name.chars().next().expect("count is one");
                if character.is_ascii_uppercase() {
                    character = character.to_ascii_lowercase();
                    modifiers.insert(KeyModifiers::SHIFT);
                }
                KeyCode::Char(character)
            }
            _ => return Err(format!("unknown key code `{code_name}` in `{value}`")),
        };
        let chord = Self { code, modifiers };
        if chord.is_emergency_interrupt() {
            return Err("Ctrl+c is reserved for the emergency interrupt".into());
        }
        Ok(chord)
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            write!(formatter, "Ctrl+")?;
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            write!(formatter, "Alt+")?;
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            write!(formatter, "Shift+")?;
        }
        match self.code {
            KeyCode::Enter => formatter.write_str("Enter"),
            KeyCode::Esc => formatter.write_str("Esc"),
            KeyCode::Char(' ') => formatter.write_str("Space"),
            KeyCode::Char(character) => write!(formatter, "{character}"),
            KeyCode::Backspace => formatter.write_str("Backspace"),
            KeyCode::Left => formatter.write_str("Left"),
            KeyCode::Right => formatter.write_str("Right"),
            KeyCode::Up => formatter.write_str("Up"),
            KeyCode::Down => formatter.write_str("Down"),
            KeyCode::PageUp => formatter.write_str("PageUp"),
            KeyCode::PageDown => formatter.write_str("PageDown"),
            KeyCode::Home => formatter.write_str("Home"),
            KeyCode::End => formatter.write_str("End"),
            KeyCode::Tab => formatter.write_str("Tab"),
            KeyCode::F(number) => write!(formatter, "F{number}"),
            _ => formatter.write_str("unsupported"),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Bindings {
    by_context: HashMap<ActionContext, HashMap<KeyChord, Action>>,
}

impl Bindings {
    pub(crate) fn built_in() -> Self {
        let mut bindings = Self {
            by_context: HashMap::new(),
        };
        for (context, action, keys) in [
            (ActionContext::Normal, Action::Quit, &["q"][..]),
            (ActionContext::Normal, Action::StartSearch, &["/"][..]),
            (ActionContext::Normal, Action::ToggleSort, &["s"][..]),
            (
                ActionContext::Normal,
                Action::ThumbnailLarger,
                &["+", "="][..],
            ),
            (ActionContext::Normal, Action::ThumbnailSmaller, &["-"][..]),
            (ActionContext::Normal, Action::ThumbnailReset, &["0"][..]),
            (ActionContext::Normal, Action::SetQuality(1), &["1"][..]),
            (ActionContext::Normal, Action::SetQuality(2), &["2"][..]),
            (ActionContext::Normal, Action::SetQuality(3), &["3"][..]),
            (ActionContext::Normal, Action::SetQuality(4), &["4"][..]),
            (ActionContext::Normal, Action::SetQuality(5), &["5"][..]),
            (ActionContext::Normal, Action::SetQuality(6), &["6"][..]),
            (ActionContext::Normal, Action::SetQuality(7), &["7"][..]),
            (ActionContext::Normal, Action::SetQuality(8), &["8"][..]),
            (ActionContext::Normal, Action::SetQuality(9), &["9"][..]),
            (ActionContext::Normal, Action::MoveLeft, &["Left", "h"][..]),
            (
                ActionContext::Normal,
                Action::MoveRight,
                &["Right", "l"][..],
            ),
            (ActionContext::Normal, Action::MoveUp, &["Up", "k"][..]),
            (ActionContext::Normal, Action::MoveDown, &["Down", "j"][..]),
            (ActionContext::Normal, Action::PageUp, &["PageUp"][..]),
            (ActionContext::Normal, Action::PageDown, &["PageDown"][..]),
            (ActionContext::Normal, Action::First, &["Home"][..]),
            (ActionContext::Normal, Action::Last, &["End"][..]),
            (ActionContext::Normal, Action::ToggleHelp, &["?"][..]),
            (ActionContext::Normal, Action::ToggleDebug, &["F12"][..]),
            (ActionContext::Normal, Action::OpenInspect, &["Enter"][..]),
            (ActionContext::Normal, Action::OpenCompare, &["c"][..]),
            (ActionContext::Normal, Action::ToggleMark, &["Space"][..]),
            (ActionContext::Normal, Action::MarkRange, &["v"][..]),
            (ActionContext::Normal, Action::ClearMarks, &["u"][..]),
            (ActionContext::Normal, Action::ToggleMarkedOnly, &["m"][..]),
            (
                ActionContext::Normal,
                Action::ConfirmSelection,
                &["Ctrl+s"][..],
            ),
            (ActionContext::Search, Action::CancelSearch, &["Esc"][..]),
            (ActionContext::Search, Action::CommitSearch, &["Enter"][..]),
            (
                ActionContext::Search,
                Action::DeleteSearchCharacter,
                &["Backspace"][..],
            ),
            (ActionContext::Inspect, Action::Quit, &["q"][..]),
            (ActionContext::Inspect, Action::CloseInspect, &["Esc"][..]),
            (ActionContext::Inspect, Action::PreviousImage, &["p"][..]),
            (ActionContext::Inspect, Action::NextImage, &["n"][..]),
            (ActionContext::Inspect, Action::ToggleFitHundred, &["z"][..]),
            (ActionContext::Inspect, Action::ZoomIn, &["+"][..]),
            (ActionContext::Inspect, Action::ZoomOut, &["-"][..]),
            (ActionContext::Inspect, Action::PanLeft, &["Left", "h"][..]),
            (
                ActionContext::Inspect,
                Action::PanRight,
                &["Right", "l"][..],
            ),
            (ActionContext::Inspect, Action::PanUp, &["Up", "k"][..]),
            (ActionContext::Inspect, Action::PanDown, &["Down", "j"][..]),
            (ActionContext::Inspect, Action::CycleBackground, &["b"][..]),
            (ActionContext::Inspect, Action::ToggleHelp, &["?"][..]),
            (ActionContext::Inspect, Action::ToggleDebug, &["F12"][..]),
            (ActionContext::Inspect, Action::ToggleMark, &["Space"][..]),
            (ActionContext::Inspect, Action::MarkRange, &["v"][..]),
            (ActionContext::Inspect, Action::ClearMarks, &["u"][..]),
            (ActionContext::Inspect, Action::ToggleMarkedOnly, &["m"][..]),
            (
                ActionContext::Inspect,
                Action::ConfirmSelection,
                &["Ctrl+s"][..],
            ),
            (ActionContext::Compare, Action::Quit, &["q"][..]),
            (ActionContext::Compare, Action::CloseInspect, &["Esc"][..]),
            (ActionContext::Compare, Action::PreviousImage, &["p"][..]),
            (ActionContext::Compare, Action::NextImage, &["n"][..]),
            (
                ActionContext::Compare,
                Action::PromoteCandidate,
                &["Enter"][..],
            ),
            (
                ActionContext::Compare,
                Action::SwitchComparePane,
                &["Tab"][..],
            ),
            (
                ActionContext::Compare,
                Action::ToggleCompareSync,
                &["s"][..],
            ),
            (ActionContext::Compare, Action::ToggleFitHundred, &["z"][..]),
            (ActionContext::Compare, Action::ZoomIn, &["+"][..]),
            (ActionContext::Compare, Action::ZoomOut, &["-"][..]),
            (ActionContext::Compare, Action::PanLeft, &["Left", "h"][..]),
            (
                ActionContext::Compare,
                Action::PanRight,
                &["Right", "l"][..],
            ),
            (ActionContext::Compare, Action::PanUp, &["Up", "k"][..]),
            (ActionContext::Compare, Action::PanDown, &["Down", "j"][..]),
            (ActionContext::Compare, Action::CycleBackground, &["b"][..]),
            (ActionContext::Compare, Action::ToggleMark, &["Space"][..]),
            (ActionContext::Compare, Action::ClearMarks, &["u"][..]),
            (ActionContext::Compare, Action::ToggleMarkedOnly, &["m"][..]),
            (ActionContext::Compare, Action::ToggleHelp, &["?"][..]),
            (ActionContext::Compare, Action::ToggleDebug, &["F12"][..]),
            (
                ActionContext::Compare,
                Action::ConfirmSelection,
                &["Ctrl+s"][..],
            ),
        ] {
            bindings
                .replace_action(context, action, keys.iter().copied())
                .expect("built-in bindings are valid");
        }
        bindings
    }

    pub(crate) fn replace_action(
        &mut self,
        context: ActionContext,
        action: Action,
        keys: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<(), String> {
        let parsed = keys
            .into_iter()
            .map(|key| key.as_ref().parse::<KeyChord>())
            .collect::<Result<Vec<_>, _>>()?;
        let map = self.by_context.entry(context).or_default();
        map.retain(|_, existing| *existing != action);
        for key in parsed {
            if let Some(existing) = map.get(&key)
                && *existing != action
            {
                return Err(format!(
                    "key `{key}` is bound to both {} and {} in {context:?}",
                    existing.label(),
                    action.label()
                ));
            }
            map.insert(key, action);
        }
        Ok(())
    }

    pub(crate) fn action(&self, context: ActionContext, event: KeyEvent) -> Option<Action> {
        self.by_context
            .get(&context)
            .and_then(|map| map.get(&KeyChord::from_event(event)))
            .copied()
    }

    pub(crate) fn help_lines(&self, context: ActionContext) -> Vec<String> {
        let Some(map) = self.by_context.get(&context) else {
            return Vec::new();
        };
        let mut by_action: HashMap<Action, Vec<String>> = HashMap::new();
        for (key, action) in map {
            by_action.entry(*action).or_default().push(key.to_string());
        }
        let mut lines = by_action
            .into_iter()
            .map(|(action, mut keys)| {
                keys.sort();
                format!("{:<24} {}", keys.join(", "), action.label())
            })
            .collect::<Vec<_>>();
        lines.sort();
        lines
    }

    pub(crate) fn keys_for(&self, context: ActionContext, action: Action) -> String {
        let mut keys = self
            .by_context
            .get(&context)
            .into_iter()
            .flat_map(|map| map.iter())
            .filter(|(_, existing)| **existing == action)
            .map(|(key, _)| key.to_string())
            .collect::<Vec<_>>();
        keys.sort();
        keys.join("/")
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyEventKind, KeyEventState};

    use super::*;

    fn event(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn built_in_bindings_preserve_existing_navigation() {
        let bindings = Bindings::built_in();
        assert_eq!(
            bindings.action(
                ActionContext::Normal,
                event(KeyCode::Char('j'), KeyModifiers::NONE)
            ),
            Some(Action::MoveDown)
        );
        assert_eq!(
            bindings.action(
                ActionContext::Normal,
                event(KeyCode::PageDown, KeyModifiers::NONE)
            ),
            Some(Action::PageDown)
        );
        assert_eq!(
            bindings.action(
                ActionContext::Normal,
                event(KeyCode::Char(' '), KeyModifiers::NONE)
            ),
            Some(Action::ToggleMark)
        );
        assert_eq!(
            bindings.action(
                ActionContext::Inspect,
                event(KeyCode::Char('s'), KeyModifiers::CONTROL)
            ),
            Some(Action::ConfirmSelection)
        );
    }

    #[test]
    fn escape_is_contextual_and_never_the_default_grid_quit_key() {
        let bindings = Bindings::built_in();
        let escape = event(KeyCode::Esc, KeyModifiers::NONE);

        assert_eq!(bindings.action(ActionContext::Normal, escape), None);
        assert_eq!(
            bindings.action(ActionContext::Search, escape),
            Some(Action::CancelSearch)
        );
        assert_eq!(
            bindings.action(ActionContext::Inspect, escape),
            Some(Action::CloseInspect)
        );
        assert_eq!(
            bindings.action(ActionContext::Compare, escape),
            Some(Action::CloseInspect)
        );

        for context in [
            ActionContext::Normal,
            ActionContext::Inspect,
            ActionContext::Compare,
        ] {
            assert_eq!(
                bindings.action(context, event(KeyCode::Char('q'), KeyModifiers::NONE)),
                Some(Action::Quit)
            );
        }
    }

    #[test]
    fn replacement_rejects_conflicts_and_reserved_interrupt() {
        let mut bindings = Bindings::built_in();
        assert!(
            bindings
                .replace_action(ActionContext::Normal, Action::MoveLeft, ["j"])
                .is_err()
        );
        assert!("Ctrl+c".parse::<KeyChord>().is_err());
    }

    #[test]
    fn sort_binding_can_be_replaced_and_help_tracks_the_active_key() {
        let mut bindings = Bindings::built_in();
        let action = Action::from_name("toggle_sort").unwrap();
        assert_eq!(
            bindings.action(
                ActionContext::Normal,
                event(KeyCode::Char('s'), KeyModifiers::NONE)
            ),
            Some(action)
        );
        bindings
            .replace_action(ActionContext::Normal, action, ["F6"])
            .unwrap();
        assert_eq!(
            bindings.action(
                ActionContext::Normal,
                event(KeyCode::Char('s'), KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            bindings.action(
                ActionContext::Normal,
                event(KeyCode::F(6), KeyModifiers::NONE)
            ),
            Some(Action::ToggleSort)
        );
        assert!(
            bindings
                .help_lines(ActionContext::Normal)
                .iter()
                .any(|line| { line.contains("F6") && line.contains("discovery / path order") })
        );
        assert_eq!(
            bindings.action(
                ActionContext::Search,
                event(KeyCode::Char('s'), KeyModifiers::NONE)
            ),
            None
        );
    }

    #[test]
    fn shifted_symbols_match_terminal_events_without_shift() {
        let bindings = Bindings::built_in();
        assert_eq!(
            bindings.action(
                ActionContext::Normal,
                event(KeyCode::Char('+'), KeyModifiers::SHIFT)
            ),
            Some(Action::ThumbnailLarger)
        );
    }

    #[test]
    fn help_is_derived_from_effective_bindings() {
        let mut bindings = Bindings::built_in();
        bindings
            .replace_action(ActionContext::Normal, Action::StartSearch, ["Ctrl+f"])
            .unwrap();
        let help = bindings.help_lines(ActionContext::Normal).join("\n");
        assert!(help.contains("Ctrl+f"));
        assert!(!help.lines().any(|line| line.starts_with('/')));
    }
}
