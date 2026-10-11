//! Updates and version switching (FLIT_SPEC §22), the agent's side.
//!
//! Releases are published on GitHub Releases by CI (Tabor's job); the
//! agent only reads a release manifest (JSON) from the URL in settings,
//! compares versions and tells the panel. Installing side-by-side versions
//! and switching the active one is the installer's job; the manifest names
//! the download for each platform. The data side (app_version in every act,
//! the song's Required version held by the keeper) lives in flit-core.

use flit_ipc::{ReleaseRow, UpdateInfo};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct ManifestRelease {
    version: String,
    #[serde(default = "stable")]
    channel: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    url: String,
}

fn stable() -> String {
    "stable".into()
}

#[derive(Debug, Deserialize)]
struct Manifest {
    releases: Vec<ManifestRelease>,
}

/// Compare dotted versions numerically ("0.10.0" > "0.9.1").
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> { s.trim_start_matches('v').split(['.', '-', '+']).map(|p| p.parse::<u64>().unwrap_or(0)).collect() };
    parse(a) > parse(b)
}

/// Read the manifest: a local file, or an http(s) URL fetched with the
/// system's curl (Windows 10+ ships one). No account, no telemetry.
fn read_manifest(source: &str) -> Result<String, String> {
    let s = source.trim();
    if s.is_empty() {
        return Err("no release manifest configured (Settings > Updates)".into());
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        let out = std::process::Command::new("curl").args(["-sSL", "--max-time", "15", s]).output().map_err(|e| format!("curl: {e}"))?;
        if !out.status.success() {
            return Err(format!("could not fetch {s}: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    std::fs::read_to_string(Path::new(s)).map_err(|e| format!("{s}: {e}"))
}

/// What the manifest says, against the running version and the installed
/// versions found in `installed`.
pub fn check(source: &str, channel: &str, current: &str, installed: &[String]) -> UpdateInfo {
    let mut info = UpdateInfo { current: current.to_string(), newest: None, channel: channel.to_string(), releases: vec![], message: String::new() };
    let text = match read_manifest(source) {
        Ok(t) => t,
        Err(e) => {
            info.message = e;
            return info;
        }
    };
    let m: Manifest = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            info.message = format!("manifest is not valid: {e}");
            return info;
        }
    };
    let mut releases: Vec<ReleaseRow> = m
        .releases
        .into_iter()
        .filter(|r| channel == "beta" || r.channel == "stable")
        .map(|r| ReleaseRow { installed: installed.iter().any(|v| v == &r.version) || r.version == current, version: r.version, channel: r.channel, notes: r.notes, url: r.url })
        .collect();
    releases.sort_by(|a, b| if newer(&a.version, &b.version) { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater });
    info.newest = releases.first().filter(|r| newer(&r.version, current)).map(|r| r.version.clone());
    info.message = match &info.newest {
        Some(v) => format!("FLit {v} is available"),
        None => "FLit is up to date".into(),
    };
    info.releases = releases;
    info
}

/// Versions installed side by side under `<data dir>/versions/<version>/`.
pub fn installed_versions(data_dir: &Path) -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(data_dir.join("versions")) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                v.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_order_and_manifest() {
        assert!(newer("0.10.0", "0.9.1"));
        assert!(!newer("0.9.1", "0.9.1"));
        assert!(newer("1.0.0", "0.99.9"));
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("releases.json");
        std::fs::write(&p, r#"{"releases":[{"version":"0.2.0","notes":"n"},{"version":"0.3.0-beta","channel":"beta"},{"version":"0.1.0"}]}"#).unwrap();
        let info = check(p.to_str().unwrap(), "stable", "0.1.0", &[]);
        assert_eq!(info.newest.as_deref(), Some("0.2.0"));
        assert_eq!(info.releases.len(), 2, "beta hidden on the stable channel");
        assert!(info.releases.iter().any(|r| r.version == "0.1.0" && r.installed));
        let info = check(p.to_str().unwrap(), "beta", "0.2.0", &[]);
        assert_eq!(info.newest.as_deref(), Some("0.3.0-beta"));
        let none = check("", "stable", "0.1.0", &[]);
        assert!(none.message.contains("no release manifest"));
    }
}
