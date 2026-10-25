// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::HashMap, fs, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub background: crate::background::Settings,
    pub history: crate::history::Settings,
    pub shortcuts: crate::shortcuts::Shortcuts,
    pub model: String,
    pub context_chars: usize,
    pub delay_ms: u64,
    pub max_words: usize,
    pub midline: bool,
    pub google_docs: bool,
    pub writing_instructions: String,
    pub blocked_apps: Vec<String>,
    pub app_rules: HashMap<String, AppRule>,
    pub website_rules: HashMap<String, WebsiteRule>,
    pub spelling: SpellingSettings,
    pub emoji: bool,
    pub choices: crate::alternatives::ChoiceSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SpellingSettings {
    pub enabled: bool,
    pub show_fixes: bool,
    pub language: String,
}
impl Default for SpellingSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            show_fixes: true,
            language: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WebsiteRule {
    pub enabled: bool,
    pub append_instructions: bool,
    pub word_shortcut: bool,
    pub writing_instructions: String,
}
impl Default for WebsiteRule {
    fn default() -> Self {
        Self {
            enabled: true,
            append_instructions: false,
            word_shortcut: true,
            writing_instructions: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppRule {
    pub enabled: bool,
    #[serde(default)]
    pub append_instructions: bool,
    #[serde(default)]
    pub complete_in_code: bool,
    #[serde(default = "enabled_by_default")]
    pub word_shortcut: bool,
    #[serde(default)]
    pub writing_instructions: String,
}
fn enabled_by_default() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            background: crate::background::Settings::default(),
            history: crate::history::Settings::default(),
            shortcuts: crate::shortcuts::Shortcuts::default(),
            model: "qwen3:1.7b".into(),
            context_chars: 1200,
            delay_ms: 250,
            max_words: 8,
            midline: true,
            google_docs: false,
            writing_instructions: String::new(),
            blocked_apps: [
                "com.apple.Terminal",
                "com.googlecode.iterm2",
                "dev.warp.Warp-Stable",
                "com.mitchellh.ghostty",
                "org.alacritty",
                "net.kovidgoyal.kitty",
                "com.agilebits.onepassword7",
                "com.1password.1password",
                "com.1password.1password7",
                "com.bitwarden.desktop",
                "com.dashlane.Dashlane",
                "com.lastpass.LastPass",
                "org.keepassxc.keepassxc",
                "com.apple.Passwords",
                "com.protonpass.macos",
                "com.libretypist.app",
                "org.libretypist.app",
                "local.libretypist",
                "com.libretypist.LibreTypist",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            app_rules: HashMap::new(),
            website_rules: HashMap::new(),
            spelling: SpellingSettings::default(),
            emoji: true,
            choices: crate::alternatives::ChoiceSettings::default(),
        }
    }
}

fn compose<'a>(inherited: Cow<'a, str>, local: &'a str, append: bool) -> Cow<'a, str> {
    if local.trim().is_empty() {
        inherited
    } else if append && !inherited.trim().is_empty() {
        Cow::Owned(format!("{inherited}\n{local}"))
    } else {
        Cow::Borrowed(local)
    }
}

impl Config {
    pub fn path() -> Result<PathBuf, String> {
        let home = std::env::var_os("HOME").ok_or("HOME is unavailable")?;
        Ok(PathBuf::from(home).join("Library/Application Support/LibreTypist/config.json"))
    }

    pub fn load() -> Result<Self, String> {
        let path = Self::path()?;
        match fs::read(path) {
            Ok(bytes) => {
                let config: Self = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid LibreType configuration: {error}"))?;
                config.validate()?;
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("Cannot read LibreType configuration: {error}")),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        self.validate()?;
        let path = Self::path()?;
        fs::create_dir_all(path.parent().ok_or("Missing configuration directory")?)
            .map_err(|error| error.to_string())?;
        // Only settings are serialized. Typed text and model responses never enter this file.
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temp, bytes).map_err(|error| error.to_string())?;
        fs::rename(temp, path).map_err(|error| error.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        self.shortcuts.validate()?;
        self.background.validate()?;
        self.history.validate()?;
        if !(250..=10000).contains(&self.choices.delay_ms) {
            return Err("Word-choice delay must be between 250 and 10000 ms.".into());
        }
        if self.spelling.language.len() > 32
            || self
                .spelling
                .language
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-')))
        {
            return Err(
                "Choose a supported spelling language, or leave it empty for the system default."
                    .into(),
            );
        }
        for host in self.website_rules.keys() {
            if crate::website::canonical_host(host).as_deref() != Ok(host.as_str()) {
                return Err("Website rules must use canonical exact domains.".into());
            }
        }
        if std::iter::once(&self.writing_instructions)
            .chain(
                self.app_rules
                    .values()
                    .map(|rule| &rule.writing_instructions),
            )
            .chain(
                self.website_rules
                    .values()
                    .map(|rule| &rule.writing_instructions),
            )
            .any(|text| {
                text.chars().count() > 1000
                    || text
                        .chars()
                        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            })
        {
            return Err(
                "Writing instructions must be at most 1,000 characters with no control codes"
                    .into(),
            );
        }
        if self.model.trim().is_empty() || self.model.chars().any(char::is_control) {
            return Err("Choose a nonempty local Ollama model name".into());
        }
        if self.model.to_ascii_lowercase().contains("cloud") {
            return Err("Cloud models are disabled; choose a downloaded local model".into());
        }
        if !(64..=4096).contains(&self.context_chars) {
            return Err("context_chars must be between 64 and 4096".into());
        }
        if !(100..=10_000).contains(&self.delay_ms) {
            return Err("delay_ms must be between 100 and 10000".into());
        }
        if !(1..=32).contains(&self.max_words) {
            return Err("max_words must be between 1 and 32".into());
        }
        Ok(())
    }

    pub fn allows_app(&self, app_id: &str) -> bool {
        if !self.enabled || app_id.is_empty() {
            return false;
        }
        if let Some(rule) = self.app_rules.get(app_id) {
            return rule.enabled;
        }
        !crate::code_editor::supported(app_id)
            && !self
                .blocked_apps
                .iter()
                .any(|blocked| blocked.eq_ignore_ascii_case(app_id))
    }
    pub fn allows_context(&self, app_id: &str, website: Option<&str>) -> bool {
        self.allows_app(app_id)
            && website.is_none_or(|host| {
                crate::website::canonical_host(host).as_deref() == Ok(host)
                    && self.website_rules.get(host).is_none_or(|rule| rule.enabled)
            })
    }
    pub fn instructions_for_context(&self, app_id: &str, website: Option<&str>) -> Cow<'_, str> {
        let inherited = self.instructions_for(app_id);
        match website.and_then(|host| self.website_rules.get(host)) {
            Some(rule) => compose(
                inherited,
                &rule.writing_instructions,
                rule.append_instructions,
            ),
            None => inherited,
        }
    }
    pub fn website_word_shortcut_enabled(&self, website: Option<&str>) -> bool {
        website
            .and_then(|host| self.website_rules.get(host))
            .is_none_or(|rule| rule.word_shortcut)
    }

    pub fn set_app_enabled(&mut self, app_id: &str, enabled: bool) {
        self.app_rules
            .entry(app_id.to_owned())
            .and_modify(|rule| rule.enabled = enabled)
            .or_insert(AppRule {
                enabled,
                append_instructions: false,
                word_shortcut: true,
                complete_in_code: false,
                writing_instructions: String::new(),
            });
    }

    /// Empty instructions inherit. Existing nonempty rules replace by default;
    /// an explicit append option layers global, app and exact-domain preferences.
    pub fn instructions_for(&self, app_id: &str) -> Cow<'_, str> {
        let inherited = Cow::Borrowed(self.writing_instructions.as_str());
        match self.app_rules.get(app_id) {
            Some(rule) => compose(
                inherited,
                &rule.writing_instructions,
                rule.append_instructions,
            ),
            None => inherited,
        }
    }
    pub fn app_appends_instructions(&self, app_id: &str) -> bool {
        self.app_rules
            .get(app_id)
            .is_some_and(|rule| rule.append_instructions)
    }
    pub fn set_app_append_instructions(&mut self, app_id: &str, append: bool) {
        if app_id.is_empty() || (!append && !self.app_rules.contains_key(app_id)) {
            return;
        }
        let enabled = !crate::code_editor::supported(app_id)
            && !self
                .blocked_apps
                .iter()
                .any(|app| app.eq_ignore_ascii_case(app_id));
        self.app_rules
            .entry(app_id.into())
            .or_insert(AppRule {
                enabled,
                append_instructions: false,
                word_shortcut: true,
                complete_in_code: false,
                writing_instructions: String::new(),
            })
            .append_instructions = append;
    }

    pub fn set_app_instructions(&mut self, app_id: &str, instructions: String) {
        if instructions.trim().is_empty() && !self.app_rules.contains_key(app_id) {
            return;
        }
        // Editing preferences never enables a blocked app, even while globally paused.
        let enabled = !crate::code_editor::supported(app_id)
            && !self
                .blocked_apps
                .iter()
                .any(|app| app.eq_ignore_ascii_case(app_id));
        self.app_rules
            .entry(app_id.to_owned())
            .or_insert(AppRule {
                enabled,
                append_instructions: false,
                word_shortcut: true,
                complete_in_code: false,
                writing_instructions: String::new(),
            })
            .writing_instructions = instructions;
    }
    pub fn complete_in_code(&self, app_id: &str) -> bool {
        crate::code_editor::supported(app_id)
            && self
                .app_rules
                .get(app_id)
                .is_some_and(|rule| rule.complete_in_code)
    }
    pub fn set_complete_in_code(&mut self, app_id: &str, enabled: bool) {
        if !crate::code_editor::supported(app_id) {
            return;
        }
        self.app_rules
            .entry(app_id.into())
            .or_insert(AppRule {
                enabled: false,
                append_instructions: false,
                word_shortcut: true,
                complete_in_code: false,
                writing_instructions: String::new(),
            })
            .complete_in_code = enabled;
    }
    pub fn word_shortcut_enabled(&self, app_id: &str) -> bool {
        self.app_rules
            .get(app_id)
            .is_none_or(|rule| rule.word_shortcut)
    }
    pub fn shortcut_action(
        &self,
        app_id: &str,
        key: u16,
        flags: u64,
    ) -> Option<crate::shortcuts::Action> {
        use crate::shortcuts::{Action, DismissBehavior};
        self.shortcuts
            .matching(key, flags)
            .filter(|action| match action {
                Action::Word => self.word_shortcut_enabled(app_id),
                Action::Dismiss => self.shortcuts.dismiss_behavior != DismissBehavior::PassThrough,
                _ => true,
            })
    }
    pub fn set_word_shortcut(&mut self, app_id: &str, enabled: bool) {
        if app_id.is_empty() || (enabled && !self.app_rules.contains_key(app_id)) {
            return;
        }
        let app_enabled = !crate::code_editor::supported(app_id)
            && !self
                .blocked_apps
                .iter()
                .any(|app| app.eq_ignore_ascii_case(app_id));
        self.app_rules
            .entry(app_id.into())
            .or_insert(AppRule {
                enabled: app_enabled,
                append_instructions: false,
                word_shortcut: true,
                complete_in_code: false,
                writing_instructions: String::new(),
            })
            .word_shortcut = enabled;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appended_instructions_resolve_global_app_and_exact_domain_in_order() {
        let mut config = Config {
            writing_instructions: "Global".into(),
            ..Config::default()
        };
        config.set_app_instructions("browser", "App".into());
        config.website_rules.insert(
            "mail.example".into(),
            WebsiteRule {
                writing_instructions: "Site".into(),
                ..Default::default()
            },
        );
        for (app, site, expected) in [
            (false, false, "Site"),
            (true, false, "Site"),
            (false, true, "App\nSite"),
            (true, true, "Global\nApp\nSite"),
        ] {
            config.set_app_append_instructions("browser", app);
            config
                .website_rules
                .get_mut("mail.example")
                .unwrap()
                .append_instructions = site;
            assert_eq!(
                config.instructions_for_context("browser", Some("mail.example")),
                expected
            );
        }
        assert_eq!(
            config.instructions_for_context("browser", Some("mail.example.evil.test")),
            "Global\nApp"
        );
        assert_eq!(
            config.instructions_for_context("other", Some("mail.example")),
            "Global\nSite"
        );
        config
            .website_rules
            .get_mut("mail.example")
            .unwrap()
            .writing_instructions = "  ".into();
        assert_eq!(
            config.instructions_for_context("browser", Some("mail.example")),
            "Global\nApp"
        );
        config.set_app_instructions("browser", "".into());
        assert_eq!(
            config.instructions_for_context("browser", Some("mail.example")),
            "Global"
        );
        config.writing_instructions.clear();
        config.set_app_instructions("browser", "App".into());
        assert_eq!(config.instructions_for("browser"), "App");
    }
    #[test]
    fn append_modes_migrate_and_roundtrip_without_enabling_apps_or_expanding_limits() {
        let old = r#"{"writing_instructions":"Global","app_rules":{"editor":{"enabled":false,"writing_instructions":"App"}},"website_rules":{"mail.example":{"writing_instructions":"Site"}}}"#;
        let mut config: Config = serde_json::from_str(old).unwrap();
        assert!(!config.app_appends_instructions("editor"));
        assert!(!config.website_rules["mail.example"].append_instructions);
        assert_eq!(config.instructions_for("editor"), "App");
        config.enabled = false;
        config.set_app_append_instructions("editor", true);
        config.set_app_append_instructions("com.apple.Terminal", true);
        config.set_app_append_instructions("com.microsoft.VSCode", true);
        for app in ["editor", "com.apple.Terminal", "com.microsoft.VSCode"] {
            assert!(!config.app_rules[app].enabled);
        }
        config
            .website_rules
            .get_mut("mail.example")
            .unwrap()
            .append_instructions = true;
        let loaded: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(
            loaded.instructions_for_context("editor", Some("mail.example")),
            "Global\nApp\nSite"
        );
        config.writing_instructions = "全".repeat(1000);
        config.set_app_instructions("editor", "語".repeat(1000));
        config
            .website_rules
            .get_mut("mail.example")
            .unwrap()
            .writing_instructions = "文".repeat(1000);
        assert!(config.validate().is_ok());
        assert_eq!(
            config
                .instructions_for_context("editor", Some("mail.example"))
                .chars()
                .count(),
            3002
        );
        config.set_app_instructions("editor", "語".repeat(1001));
        assert!(config.validate().is_err());
    }
    #[test]
    fn code_editor_preferences_migrate_without_enabling_an_excluded_app() {
        let app = "com.microsoft.VSCode";
        let mut c: Config = serde_json::from_str("{}").unwrap();
        assert!(!c.allows_app(app));
        c.set_app_instructions(app, "Be concise.".into());
        c.set_word_shortcut(app, false);
        c.set_complete_in_code(app, true);
        assert!(!c.allows_app(app));
        c.set_app_enabled(app, true);
        assert!(c.allows_app(app));
        assert!(c.complete_in_code(app));
        assert!(!c.word_shortcut_enabled(app));
        assert_eq!(c.instructions_for(app), "Be concise.");
        let saved = serde_json::to_string(&c).unwrap();
        let restored: Config = serde_json::from_str(&saved).unwrap();
        assert!(restored.complete_in_code(app));
        c.set_complete_in_code("com.apple.TextEdit", true);
        assert!(!c.app_rules.contains_key("com.apple.TextEdit"));
        let old = r#"{"app_rules":{"com.microsoft.VSCode":{"enabled":true,"writing_instructions":"custom"}}}"#;
        let old: Config = serde_json::from_str(old).unwrap();
        assert!(old.allows_app(app));
        assert!(!old.complete_in_code(app));
        assert_eq!(old.instructions_for(app), "custom");
        c.enabled = false;
        assert!(!c.allows_app(app));
    }
    #[test]
    fn vscodium_has_independent_app_and_source_opt_ins() {
        let mut config: Config = serde_json::from_str("{}").unwrap();
        assert!(!config.allows_app("com.vscodium"));
        config.set_complete_in_code("com.vscodium", true);
        assert!(!config.allows_app("com.vscodium"));
        config.set_app_enabled("com.vscodium", true);
        assert!(config.allows_app("com.vscodium"));
        assert!(config.complete_in_code("com.vscodium"));
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(restored.allows_app("com.vscodium"));
        assert!(restored.complete_in_code("com.vscodium"));
        config.set_complete_in_code("com.vscodium", false);
        assert!(config.allows_app("com.vscodium"));
        assert!(!config.complete_in_code("com.vscodium"));
    }
    #[test]
    fn website_rules_are_exact_and_do_not_override_app_exclusions() {
        let mut config: Config = serde_json::from_str("{}").unwrap();
        assert!(config.website_rules.is_empty());
        config.writing_instructions = "Global".into();
        config.set_app_instructions("browser", "App".into());
        config.website_rules.insert(
            "mail.google.com".into(),
            WebsiteRule {
                enabled: true,
                append_instructions: false,
                word_shortcut: false,
                writing_instructions: "Site".into(),
            },
        );
        assert_eq!(
            config.instructions_for_context("browser", Some("mail.google.com")),
            "Site"
        );
        assert_eq!(
            config.instructions_for_context("browser", Some("docs.google.com")),
            "App"
        );
        assert_eq!(
            config.instructions_for_context("browser", Some("mail.google.com.evil.example")),
            "App"
        );
        assert_eq!(config.instructions_for_context("other", None), "Global");
        assert!(!config.website_word_shortcut_enabled(Some("mail.google.com")));
        assert!(config.website_word_shortcut_enabled(Some("docs.google.com")));
        config
            .website_rules
            .get_mut("mail.google.com")
            .unwrap()
            .writing_instructions
            .clear();
        assert_eq!(
            config.instructions_for_context("browser", Some("mail.google.com")),
            "App"
        );
        config.set_app_enabled("browser", false);
        assert!(!config.allows_context("browser", Some("mail.google.com")));
        config.set_app_enabled("browser", true);
        config
            .website_rules
            .get_mut("mail.google.com")
            .unwrap()
            .enabled = false;
        assert!(!config.allows_context("browser", Some("mail.google.com")));
        assert!(config.allows_context("browser", Some("docs.google.com")));
        assert!(!config.allows_context("browser", Some("https://mail.google.com")));
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(!restored.allows_context("browser", Some("mail.google.com")));
        assert!(restored.validate().is_ok());
        config
            .website_rules
            .get_mut("mail.google.com")
            .unwrap()
            .writing_instructions = "x".repeat(1001);
        assert!(config.validate().is_err());
    }
    #[test]
    fn per_app_word_shortcut_migrates_and_preserves_other_preferences() {
        let mut c: Config = serde_json::from_str(
            r#"{"app_rules":{"editor":{"enabled":false,"writing_instructions":"Keep it brief"}}}"#,
        )
        .unwrap();
        assert!(c.word_shortcut_enabled("editor"));
        c.set_word_shortcut("editor", false);
        c.set_app_enabled("editor", true);
        c.set_app_instructions("editor", "Be friendly".into());
        assert!(!c.word_shortcut_enabled("editor"));
        assert!(c.word_shortcut_enabled("other"));
        assert_eq!(c.shortcut_action("editor", 48, 0), None);
        assert_eq!(
            c.shortcut_action("other", 48, 0),
            Some(crate::shortcuts::Action::Word)
        );
        assert_eq!(
            c.shortcut_action("editor", 48, crate::shortcuts::SHIFT),
            Some(crate::shortcuts::Action::All)
        );
        c.shortcuts.dismiss_behavior = crate::shortcuts::DismissBehavior::PassThrough;
        assert_eq!(c.shortcut_action("editor", 53, 0), None);
        c.shortcuts.word = Some(crate::shortcuts::Hotkey::new(12, crate::shortcuts::CONTROL));
        assert_eq!(
            c.shortcut_action("editor", 12, crate::shortcuts::CONTROL),
            None
        );
        assert_eq!(
            c.shortcut_action("other", 12, crate::shortcuts::CONTROL),
            Some(crate::shortcuts::Action::Word)
        );
        assert_eq!(
            c.shortcuts.matching(48, crate::shortcuts::SHIFT),
            Some(crate::shortcuts::Action::All)
        );
        c.enabled = false;
        c.set_word_shortcut("com.apple.Terminal", false);
        c.enabled = true;
        assert!(!c.allows_app("com.apple.Terminal"));
        let c: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert!(!c.word_shortcut_enabled("editor"));
        assert_eq!(c.instructions_for("editor"), "Be friendly");
    }

    #[test]
    fn app_preferences_inherit_override_and_preserve_exclusions() {
        let mut config: Config = serde_json::from_str(r#"{"writing_instructions":"Global style","app_rules":{"com.apple.TextEdit":{"enabled":false}}}"#).unwrap();
        assert_eq!(
            config.instructions_for("com.apple.TextEdit"),
            "Global style"
        );
        config.set_app_instructions("com.apple.TextEdit", "App style".into());
        assert!(!config.allows_app("com.apple.TextEdit"));
        assert_eq!(config.instructions_for("com.apple.TextEdit"), "App style");
        assert_eq!(
            config.instructions_for("org.whispersystems.signal-desktop"),
            "Global style"
        );
        config.set_app_enabled("com.apple.TextEdit", true);
        assert_eq!(config.instructions_for("com.apple.TextEdit"), "App style");
        config.enabled = false;
        config.set_app_instructions("com.apple.Terminal", "Terminal style".into());
        config.enabled = true;
        assert!(!config.allows_app("com.apple.Terminal"));
        config.set_app_instructions("com.apple.TextEdit", "  ".into());
        assert_eq!(
            config.instructions_for("com.apple.TextEdit"),
            "Global style"
        );
        config.set_app_instructions("com.apple.TextEdit", "x".repeat(1001));
        assert!(config.validate().is_err());
        config.set_app_instructions("com.apple.TextEdit", "App style".into());
        let loaded: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(loaded.instructions_for("com.apple.TextEdit"), "App style");
        assert!(!loaded.allows_app("com.apple.Terminal"));
    }

    #[test]
    fn exclusions_and_explicit_overrides() {
        let mut config = Config::default();
        assert!(!config.allows_app("com.apple.Terminal"));
        assert!(!config.allows_app("com.apple.Passwords"));
        assert!(!config.allows_app(""));
        assert!(config.allows_app("com.apple.TextEdit"));
        config.set_app_enabled("com.apple.TextEdit", false);
        assert!(!config.allows_app("com.apple.TextEdit"));
        config.set_app_enabled("com.apple.Terminal", true);
        assert!(config.allows_app("com.apple.Terminal"));
        config.enabled = false;
        assert!(!config.allows_app("com.apple.Terminal"));
    }

    #[test]
    fn word_choice_modes_migrate_and_validate() {
        use crate::alternatives::RevealMode;
        let mut config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config.choices.alternatives, RevealMode::Off);
        assert_eq!(config.choices.synonyms, RevealMode::Automatic);
        assert_eq!(config.choices.delay_ms, 800);
        config.choices.alternatives = RevealMode::Shortcut;
        config.choices.synonyms = RevealMode::Off;
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(restored.choices.alternatives, RevealMode::Shortcut);
        assert_eq!(restored.choices.synonyms, RevealMode::Off);
        assert!(restored.validate().is_ok());
        config.choices.delay_ms = 249;
        assert!(config.validate().is_err());
        config.choices.delay_ms = 10001;
        assert!(config.validate().is_err());
    }

    #[test]
    fn spelling_settings_migrate_and_validate() {
        let mut config: Config = serde_json::from_str("{}").unwrap();
        assert!(config.spelling.enabled && config.spelling.show_fixes);
        assert!(config.spelling.language.is_empty());
        config.spelling = SpellingSettings {
            enabled: false,
            show_fixes: false,
            language: "en_US".into(),
        };
        assert!(config.validate().is_ok());
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(!restored.spelling.enabled && !restored.spelling.show_fixes);
        assert_eq!(restored.spelling.language, "en_US");
        config.spelling.language = "en\nUS".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn docs_companion_requires_explicit_opt_in_and_roundtrips() {
        let mut config: Config = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(!config.google_docs);
        config.google_docs = true;
        let restored: Config = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(restored.google_docs);
        config.google_docs = false;
        assert!(!serde_json::from_str::<Config>(&serde_json::to_string(&config).unwrap()).unwrap().google_docs);
    }

    #[test]
    fn settings_roundtrip_and_defaults() {
        let config = Config {
            writing_instructions: "Use British English.\nKeep the tone friendly.".into(),
            ..Config::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(
            serde_json::from_str::<Config>(&json).unwrap().model,
            config.model
        );
        assert_eq!(serde_json::from_str::<Config>("{}").unwrap().delay_ms, 250);
        assert!(serde_json::from_str::<Config>("{}").unwrap().emoji);
        let disabled: Config = serde_json::from_str(r#"{"emoji":false}"#).unwrap();
        assert!(
            !serde_json::from_str::<Config>(&serde_json::to_string(&disabled).unwrap())
                .unwrap()
                .emoji
        );
        assert!(serde_json::from_str::<Config>("{}")
            .unwrap()
            .writing_instructions
            .is_empty());
        assert_eq!(
            serde_json::from_str::<Config>(&json)
                .unwrap()
                .writing_instructions,
            config.writing_instructions
        );
        assert!(config.validate().is_ok());
        assert!(Config {
            writing_instructions: "x".repeat(1001),
            ..Config::default()
        }
        .validate()
        .is_err());
    }
}
