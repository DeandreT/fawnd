//! Window → profile rules for automatic profile switching.
//!
//! Loaded from `$XDG_CONFIG_HOME/fawnd/rules.toml`:
//!
//! ```toml
//! # Profile to use when no rule matches (optional).
//! default = "typing"
//!
//! # First matching rule wins. `match` is a glob (`*` wildcard) tested
//! # case-insensitively against the focused window's app-id / resource class.
//! [[rule]]
//! match = "steam_app_*"
//! profile = "gaming"
//!
//! [[rule]]
//! match = "*.exe"
//! profile = "gaming"
//! ```

use std::path::PathBuf;

use serde::Deserialize;

use crate::error::{Error, Result};

/// Configuration directory for fawnd (`$XDG_CONFIG_HOME/fawnd`).
pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("fawnd")
}

fn rules_path() -> PathBuf {
    config_dir().join("rules.toml")
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rules {
    /// Profile applied when no rule matches.
    #[serde(default)]
    pub default: Option<String>,
    /// Ordered match rules; first match wins.
    #[serde(default, rename = "rule")]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    /// Glob pattern (`*` wildcard) matched against the window app-id.
    #[serde(rename = "match")]
    pub pattern: String,
    /// Profile to apply when this rule matches.
    pub profile: String,
}

impl Rules {
    /// Load rules from the config dir, or `None` if the file doesn't exist.
    pub fn load() -> Result<Option<Rules>> {
        let path = rules_path();
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)?;
        let rules = toml::from_str(&text).map_err(|e| Error::Config(e.to_string()))?;
        Ok(Some(rules))
    }

    /// Resolve the profile for a focused window's app-id. Returns the first
    /// matching rule's profile, falling back to `default`.
    pub fn profile_for(&self, app_id: &str) -> Option<&str> {
        let id = app_id.to_lowercase();
        self.rules
            .iter()
            .find(|r| glob_match(&r.pattern.to_lowercase(), &id))
            .map(|r| r.profile.as_str())
            .or(self.default.as_deref())
    }
}

/// Minimal glob matcher supporting only the `*` wildcard (matches any run of
/// characters, including none). Inputs are expected to be already lowercased.
fn glob_match(pattern: &str, text: &str) -> bool {
    fn go(p: &[u8], t: &[u8]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some(b'*') => go(&p[1..], t) || (!t.is_empty() && go(p, &t[1..])),
            Some(&c) => !t.is_empty() && t[0] == c && go(&p[1..], &t[1..]),
        }
    }
    go(pattern.as_bytes(), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("steam_app_*", "steam_app_730"));
        assert!(glob_match("*.exe", "game.exe"));
        assert!(glob_match("*cs2*", "valve-cs2-client"));
        assert!(glob_match("firefox", "firefox"));
        assert!(!glob_match("firefox", "firefoxx"));
        assert!(!glob_match("steam_app_*", "steam"));
    }

    #[test]
    fn resolves_profile() {
        let rules = Rules {
            default: Some("typing".into()),
            rules: vec![
                Rule {
                    pattern: "steam_app_*".into(),
                    profile: "gaming".into(),
                },
                Rule {
                    pattern: "*.exe".into(),
                    profile: "gaming".into(),
                },
            ],
        };
        assert_eq!(rules.profile_for("steam_app_730"), Some("gaming"));
        assert_eq!(rules.profile_for("Game.EXE"), Some("gaming"));
        assert_eq!(rules.profile_for("kitty"), Some("typing"));
    }

    #[test]
    fn no_default_returns_none() {
        let rules = Rules::default();
        assert_eq!(rules.profile_for("anything"), None);
    }
}
