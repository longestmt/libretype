// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Shortcut matching is independent of keyboard layout and ignores lock/device flags.
use serde::{Deserialize, Serialize};

pub const SHIFT: u64 = 1 << 17;
pub const CONTROL: u64 = 1 << 18;
pub const OPTION: u64 = 1 << 19;
pub const COMMAND: u64 = 1 << 20;
pub const MODIFIERS: u64 = SHIFT | CONTROL | OPTION | COMMAND;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    pub key: u16,
    pub modifiers: u64,
}
impl Hotkey {
    pub fn new(key: u16, flags: u64) -> Self {
        Self {
            key,
            modifiers: flags & MODIFIERS,
        }
    }
    pub fn matches(self, key: u16, flags: u64) -> bool {
        self == Self::new(key, flags)
    }
    pub fn label(self) -> String {
        let mut text = String::new();
        for (mask, label) in [(CONTROL, "⌃"), (OPTION, "⌥"), (SHIFT, "⇧"), (COMMAND, "⌘")] {
            if self.modifiers & mask != 0 {
                text.push_str(label);
            }
        }
        // Physical-key labels: layout-independent matching, as in macOS shortcut recorders.
        let name = match self.key {
            0 => "A",
            1 => "S",
            2 => "D",
            3 => "F",
            4 => "H",
            5 => "G",
            6 => "Z",
            7 => "X",
            8 => "C",
            9 => "V",
            11 => "B",
            12 => "Q",
            13 => "W",
            14 => "E",
            15 => "R",
            16 => "Y",
            17 => "T",
            18 => "1",
            19 => "2",
            20 => "3",
            21 => "4",
            22 => "6",
            23 => "5",
            24 => "=",
            25 => "9",
            26 => "7",
            27 => "−",
            28 => "8",
            29 => "0",
            30 => "]",
            31 => "O",
            32 => "U",
            33 => "[",
            34 => "I",
            35 => "P",
            36 => "Return",
            37 => "L",
            38 => "J",
            39 => "'",
            40 => "K",
            41 => ";",
            42 => "\\",
            43 => ",",
            44 => "/",
            45 => "N",
            46 => "M",
            47 => ".",
            48 => "Tab",
            49 => "Space",
            50 => "`",
            51 => "Delete",
            53 => "Esc",
            76 => "Enter",
            96 => "F5",
            97 => "F6",
            98 => "F7",
            99 => "F3",
            100 => "F8",
            101 => "F9",
            103 => "F11",
            109 => "F10",
            111 => "F12",
            118 => "F4",
            120 => "F2",
            122 => "F1",
            123 => "←",
            124 => "→",
            125 => "↓",
            126 => "↑",
            _ => return format!("{text}Key {}", self.key),
        };
        text.push_str(name);
        text
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Word,
    All,
    Dismiss,
    Manual,
    PauseApp,
    ToggleGlobal,
    Choices,
}
pub const ACTIONS: [Action; 7] = [
    Action::Word,
    Action::All,
    Action::Dismiss,
    Action::Manual,
    Action::PauseApp,
    Action::ToggleGlobal,
    Action::Choices,
];
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Word => "Accept next word",
            Self::All => "Accept full suggestion",
            Self::Dismiss => "Dismiss suggestion",
            Self::Manual => "Request a suggestion",
            Self::PauseApp => "Pause app for 5 minutes / resume",
            Self::ToggleGlobal => "Toggle suggestions everywhere",
            Self::Choices => "Show word choices / synonyms",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DismissBehavior {
    #[default]
    PauseField,
    UntilAppSwitch,
    PassThrough,
}

/// Keeps transient pauses independent of typing and suggestion cancellation.
/// Field identity is supplied by the native accessibility adapter, never text.
#[derive(Default)]
pub struct Dismissal {
    app: String,
    until: Option<std::time::Instant>,
    active: bool,
}
impl Dismissal {
    pub fn start(
        &mut self,
        behavior: DismissBehavior,
        app: &str,
        seconds: u64,
        now: std::time::Instant,
    ) {
        self.app = app.into();
        self.active = behavior != DismissBehavior::PassThrough;
        self.until = (behavior == DismissBehavior::PauseField)
            .then(|| now + std::time::Duration::from_secs(seconds));
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn observe_app(&mut self, app: &str, now: std::time::Instant) {
        if self.app != app || self.until.is_some_and(|until| now >= until) {
            self.reset();
        }
    }
    pub fn blocks(&self, same_field: bool) -> bool {
        self.active && (self.until.is_none() || same_field)
    }
    pub fn active(&self) -> bool {
        self.active
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    pub word: Option<Hotkey>,
    pub all: Option<Hotkey>,
    pub dismiss: Option<Hotkey>,
    pub manual: Option<Hotkey>,
    pub pause_app: Option<Hotkey>,
    pub toggle_global: Option<Hotkey>,
    pub choices: Option<Hotkey>,
    pub trailing_space: bool,
    pub dismiss_behavior: DismissBehavior,
    pub dismiss_seconds: u64,
}
impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            word: Some(Hotkey::new(48, 0)),
            all: Some(Hotkey::new(48, SHIFT)),
            dismiss: Some(Hotkey::new(53, 0)),
            manual: Some(Hotkey::new(49, CONTROL | OPTION)),
            pause_app: Some(Hotkey::new(35, CONTROL | OPTION | COMMAND)),
            toggle_global: None,
            choices: Some(Hotkey::new(0, CONTROL | OPTION)),
            trailing_space: false,
            dismiss_behavior: DismissBehavior::PauseField,
            dismiss_seconds: 3,
        }
    }
}
impl Shortcuts {
    pub fn get(&self, action: Action) -> Option<Hotkey> {
        match action {
            Action::Word => self.word,
            Action::All => self.all,
            Action::Dismiss => self.dismiss,
            Action::Manual => self.manual,
            Action::PauseApp => self.pause_app,
            Action::ToggleGlobal => self.toggle_global,
            Action::Choices => self.choices,
        }
    }
    pub fn set(&mut self, action: Action, hotkey: Option<Hotkey>) {
        *match action {
            Action::Word => &mut self.word,
            Action::All => &mut self.all,
            Action::Dismiss => &mut self.dismiss,
            Action::Manual => &mut self.manual,
            Action::PauseApp => &mut self.pause_app,
            Action::ToggleGlobal => &mut self.toggle_global,
            Action::Choices => &mut self.choices,
        } = hotkey;
    }
    pub fn matching(&self, key: u16, flags: u64) -> Option<Action> {
        ACTIONS
            .into_iter()
            .find(|action| self.get(*action).is_some_and(|h| h.matches(key, flags)))
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=60).contains(&self.dismiss_seconds) {
            return Err("Dismissal pause must be between 1 and 60 seconds.".into());
        }
        let mut used = Vec::new();
        for action in ACTIONS {
            if let Some(hotkey) = self.get(action) {
                if hotkey.key >= 128 || hotkey.modifiers & !MODIFIERS != 0 {
                    return Err("Choose a valid keyboard shortcut.".into());
                }
                if hotkey.modifiers & (CONTROL | OPTION | COMMAND) == 0
                    && !matches!(
                        hotkey.key,
                        48 | 50 | 53 | 96..=101 | 103 | 109 | 111 | 118 | 120 | 122
                    )
                {
                    return Err(
                        "Use Control, Option or Command with letters, numbers and navigation keys."
                            .into(),
                    );
                }
                if hotkey.key == 48 && hotkey.modifiers & !SHIFT == OPTION {
                    return Err("Option-Tab is reserved for moving between fields.".into());
                }
                if matches!(
                    action,
                    Action::Manual | Action::PauseApp | Action::ToggleGlobal | Action::Choices
                ) && hotkey.modifiers & (CONTROL | OPTION | COMMAND) == 0
                    && !matches!(hotkey.key, 96..=101 | 103 | 109 | 111 | 118 | 120 | 122)
                {
                    return Err(
                        "Use a modified key or a function key for manual requests and toggles."
                            .into(),
                    );
                }
                if used.contains(&hotkey) {
                    return Err("Each action needs a different shortcut.".into());
                }
                used.push(hotkey);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dismissal_expires_without_typing_and_is_scoped_to_field_or_app() {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        let mut pause = Dismissal::default();
        pause.start(DismissBehavior::PauseField, "editor", 3, now);
        pause.observe_app("editor", now + Duration::from_secs(2));
        assert!(pause.blocks(true)); // Typing does not release a field pause.
        assert!(!pause.blocks(false)); // Another field remains usable.
        pause.observe_app("editor", now + Duration::from_secs(3));
        assert!(!pause.blocks(true));
        pause.start(DismissBehavior::UntilAppSwitch, "editor", 3, now);
        pause.observe_app("editor", now + Duration::from_secs(300));
        assert!(pause.blocks(false));
        pause.observe_app("browser", now);
        pause.observe_app("editor", now);
        assert!(!pause.active()); // Returning to the app must not restore the pause.
        pause.start(DismissBehavior::PassThrough, "editor", 3, now);
        assert!(!pause.blocks(true));
        pause.start(DismissBehavior::PauseField, "editor", 3, now);
        pause.reset(); // Manual activation overrides only transient pauses.
        assert!(!pause.blocks(true));
    }
    #[test]
    fn dismissal_preferences_migrate_and_validate() {
        let mut shortcuts: Shortcuts = serde_json::from_str("{}").unwrap();
        assert_eq!(shortcuts.dismiss_behavior, DismissBehavior::PauseField);
        assert_eq!(shortcuts.dismiss_seconds, 3);
        for seconds in [0, 61, u64::MAX] {
            shortcuts.dismiss_seconds = seconds;
            assert!(shortcuts.validate().is_err());
        }
        shortcuts.dismiss_seconds = 60;
        shortcuts.dismiss_behavior = DismissBehavior::PassThrough;
        assert!(shortcuts.validate().is_ok());
        let restored: Shortcuts =
            serde_json::from_str(&serde_json::to_string(&shortcuts).unwrap()).unwrap();
        assert_eq!(restored.dismiss_behavior, DismissBehavior::PassThrough);
    }
    #[test]
    fn shortcuts_require_exact_modifiers_but_ignore_caps_lock() {
        let shortcuts = Shortcuts::default();
        assert_eq!(shortcuts.matching(48, 1 << 16), Some(Action::Word));
        assert_eq!(shortcuts.matching(48, SHIFT), Some(Action::All));
        assert_eq!(shortcuts.matching(48, OPTION), None);
        assert_eq!(
            shortcuts.matching(49, CONTROL | OPTION),
            Some(Action::Manual)
        );
        assert_eq!(shortcuts.matching(49, CONTROL | OPTION | SHIFT), None);
    }
    #[test]
    fn rejects_conflicts_typing_keys_and_reserved_bypass() {
        let mut shortcuts = Shortcuts::default();
        assert!(shortcuts.validate().is_ok());
        shortcuts.manual = shortcuts.word;
        assert!(shortcuts.validate().is_err());
        shortcuts.manual = Some(Hotkey::new(0, SHIFT));
        assert!(shortcuts.validate().is_err());
        shortcuts.manual = Some(Hotkey::new(48, OPTION));
        assert!(shortcuts.validate().is_err());
        shortcuts.manual = Some(Hotkey::new(96, 0));
        assert!(shortcuts.validate().is_ok());
        shortcuts.set(Action::Manual, None);
        assert_eq!(shortcuts.get(Action::Manual), None);
        assert_eq!(Hotkey::new(48, SHIFT).label(), "⇧Tab");
        let restored: Shortcuts = serde_json::from_str("{}").unwrap();
        assert_eq!(restored.word, Shortcuts::default().word);
    }
}
