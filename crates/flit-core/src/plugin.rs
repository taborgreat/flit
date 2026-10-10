//! Host-neutral plugin identity and parameter addressing.
//!
//! A VST3 parameter is the same parameter in any host, so two people with
//! the same plugin at the same version can share settings across DAWs.

use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, PartialOrd, Ord)]
pub enum PluginFormat {
    Vst3,
    Clap,
    Au,
    /// The host's own built-in device type (its bridge names it).
    Native,
    Other(String),
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, PartialOrd, Ord)]
pub struct PluginIdentity {
    pub format: PluginFormat,
    pub vendor: String,
    pub name: String,
    pub version: String,
}

impl PluginIdentity {
    pub fn new(format: PluginFormat, vendor: &str, name: &str, version: &str) -> Self {
        Self {
            format,
            vendor: vendor.to_string(),
            name: name.to_string(),
            version: version.to_string(),
        }
    }

    pub fn vst3(vendor: &str, name: &str, version: &str) -> Self {
        Self::new(PluginFormat::Vst3, vendor, name, version)
    }

    pub fn clap(vendor: &str, name: &str, version: &str) -> Self {
        Self::new(PluginFormat::Clap, vendor, name, version)
    }

    /// Same plugin and version: settings are interchangeable.
    pub fn compatible_with(&self, other: &PluginIdentity) -> bool {
        self == other
    }
}

impl std::fmt::Display for PluginIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {} ({:?})", self.vendor, self.name, self.version, self.format)
    }
}

/// The plugin's own parameter id (VST3 ParamID / CLAP param id), as text so
/// hosts with string ids fit too.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, PartialOrd, Ord)]
pub struct ParamId(pub String);

impl ParamId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn from_u32(id: u32) -> Self {
        Self(id.to_string())
    }
}

impl From<&str> for ParamId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl std::fmt::Display for ParamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which device slot on a lane holds the plugin (0-based). Slot 0 is the
/// lane's instrument/generator, 1.. are effects, by convention.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, PartialOrd, Ord, Default)]
pub struct DeviceSlot(pub u32);
