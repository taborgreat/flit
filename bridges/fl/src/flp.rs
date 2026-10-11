//! Level 2: read an .flp through the Python helper `flp/flit_flp.py`
//! (PyFLP, GPL-3, kept in its own process and never linked). The helper
//! prints host-neutral JSON; this module turns it into a `ProjectSnapshot`.

use flit_core::{Note, SongPos, TrackId, TICKS_PER_BEAT};
use flit_host::ProjectSnapshot;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct FlpNote {
    pub position: i64,
    pub length: i64,
    pub key: i32,
    pub velocity: i32,
    pub channel: i32,
}

#[derive(Debug, Deserialize)]
pub struct FlpPattern {
    pub iid: i32,
    pub name: Option<String>,
    pub notes: Vec<FlpNote>,
}

#[derive(Debug, Deserialize)]
pub struct FlpChannel {
    pub iid: i32,
    pub name: Option<String>,
    pub kind: String,
    pub insert: Option<i32>,
    pub plugin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FlpSlot {
    pub index: i32,
    pub name: Option<String>,
    pub plugin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FlpInsert {
    pub iid: i32,
    pub name: Option<String>,
    pub volume: i64,
    pub slots: Vec<FlpSlot>,
}

#[derive(Debug, Deserialize)]
pub struct FlpSnapshot {
    pub version: String,
    pub ppq: i32,
    pub tempo: f64,
    pub channels: Vec<FlpChannel>,
    pub patterns: Vec<FlpPattern>,
    pub inserts: Vec<FlpInsert>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Where the helper script is: `FLIT_FLP_HELPER`, next to the running
/// executable (installed layout), else the source tree.
pub fn helper_path() -> PathBuf {
    if let Ok(p) = std::env::var("FLIT_FLP_HELPER") {
        return PathBuf::from(p);
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
        let p = dir.join("flit_flp.py");
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("flp").join("flit_flp.py")
}

/// A Python that can import pyflp, for level 2: `FLIT_PYTHON`, the
/// installed venv (`%LOCALAPPDATA%\flit\venv`), a venv next to the
/// executable, the repo's `.venv`, then `python` on PATH. Each candidate is
/// tried once; none = level 1 only.
pub fn find_python() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("FLIT_PYTHON") {
        candidates.push(PathBuf::from(p));
    }
    let exe_name = if cfg!(windows) { "Scripts/python.exe" } else { "bin/python" };
    if let Some(d) = dirs::data_local_dir() {
        candidates.push(d.join("flit").join("venv").join(exe_name));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
        candidates.push(dir.join("venv").join(exe_name));
        candidates.push(dir.join("..").join("venv").join(exe_name));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".venv").join(exe_name));
    candidates.push(PathBuf::from(if cfg!(windows) { "python.exe" } else { "python3" }));
    candidates.into_iter().find(|p| has_pyflp(p))
}

fn has_pyflp(python: &Path) -> bool {
    if python.is_absolute() && !python.is_file() {
        return false;
    }
    std::process::Command::new(python)
        .args(["-I", "-c", "import pyflp"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn run_helper(python: &Path, flp: &Path) -> std::io::Result<FlpSnapshot> {
    let out = std::process::Command::new(python).arg("-I").arg(helper_path()).arg(flp).output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(String::from_utf8_lossy(&out.stderr).into_owned()));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// FL ticks (PPQ per quarter) -> our ticks.
fn ticks(fl: i64, ppq: i32) -> i64 {
    fl * TICKS_PER_BEAT / ppq.max(1) as i64
}

pub fn to_snapshot(s: &FlpSnapshot) -> ProjectSnapshot {
    let mut snap = ProjectSnapshot { tempo: Some(s.tempo), files: s.files.iter().map(PathBuf::from).collect(), ..Default::default() };
    // Notes live on channels: each channel-rack channel is a track of its
    // own (`fl-channel-<global index>`), the same track the live bridge uses
    // for the channel's generator and volume.
    for p in &s.patterns {
        let mut by_track: std::collections::BTreeMap<i32, Vec<Note>> = Default::default();
        for n in &p.notes {
            by_track.entry(n.channel).or_default().push(Note {
                start: SongPos::from_ticks(ticks(n.position, s.ppq)),
                length_ticks: ticks(n.length.max(1), s.ppq),
                key: n.key.clamp(0, 127) as u8,
                velocity: n.velocity.clamp(0, 127) as u8,
                channel: 0,
            });
        }
        let clip = p.name.clone().unwrap_or_else(|| format!("Pattern {}", p.iid));
        for (chan, notes) in by_track {
            let track = crate::FlBridge::track_for_channel(chan);
            snap.arrangement.push((track.clone(), clip.clone(), SongPos::START));
            snap.notes.push((track, clip.clone(), notes));
        }
    }
    snap
}

pub fn read_project(python: &Path, flp: &Path) -> std::io::Result<ProjectSnapshot> {
    Ok(to_snapshot(&run_helper(python, flp)?))
}

#[allow(dead_code)]
fn _track(_: TrackId) {}
