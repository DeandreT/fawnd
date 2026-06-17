//! On-disk profiles: a declarative description of a keyboard configuration that
//! the controller can apply in one shot.
//!
//! Serialized as TOML, e.g.:
//!
//! ```toml
//! # Global actuation default, in millimetres.
//! actuation = 1.5
//! rapid_trigger = true
//! turbo = false
//!
//! # Per-key overrides (key name -> actuation mm).
//! [keys]
//! W = 1.2
//! A = 1.2
//! S = 1.2
//! D = 1.2
//! ```

use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::controller::Controller;
#[cfg(not(target_arch = "wasm32"))]
use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Global default actuation, in millimetres.
    #[serde(default = "default_actuation")]
    pub actuation: f32,
    #[serde(default)]
    pub rapid_trigger: bool,
    #[serde(default)]
    pub turbo: bool,
    /// Per-key actuation overrides (key name -> mm).
    #[serde(default)]
    pub keys: BTreeMap<String, f32>,
}

fn default_actuation() -> f32 {
    2.0
}

impl Default for Profile {
    fn default() -> Profile {
        Profile {
            actuation: default_actuation(),
            rapid_trigger: false,
            turbo: false,
            keys: BTreeMap::new(),
        }
    }
}

impl Profile {
    /// Load a profile from a TOML file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(path: &Path) -> Result<Profile> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::Config(e.to_string()))?;
        toml::from_str(&text).map_err(|e| Error::Config(e.to_string()))
    }

    /// Save a profile to a TOML file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        std::fs::write(path, text).map_err(|e| Error::Config(e.to_string()))
    }

    /// Apply this profile to the keyboard.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn apply(&self, ctl: &mut Controller) -> Result<()> {
        ctl.set_actuation_all(self.actuation);
        for (name, mm) in &self.keys {
            ctl.set_actuation(&[name.as_str()], *mm)?;
        }
        ctl.flush_actuation()?;
        ctl.set_rapid_trigger(self.rapid_trigger, self.turbo)?;
        Ok(())
    }
}
