//! Storage: the spine (update pack §24). Truth is a small number of
//! append-only files; everything else is a rebuildable sidecar.
//!
//! ```text
//! .flit/format                 data format version (one number)
//! .flit/spines/<name>.tree     one spine per Name: every act authored by any
//!                              of that Name's beings, in arrival order here
//! .flit/spines/keeper.tree     the keeper's spine: every fact, seal order
//! .flit/blobs/                 content-addressed files (see blob.rs)
//! .flit/sidecar/               derived, deletable: received_at notes
//! ```
//! One frame per line (JSONL): `{"h":<height>,"p":"<hash of the previous
//! frame>",<"act"|"fact">:{...}}`. The head is the last complete line. A
//! torn tail (incomplete line, or a line whose link does not match) is
//! skipped on open. Witness data (received_at) lives in the sidecar and is
//! never part of a frame.

use crate::act::*;
use crate::blob::Hash;
use crate::id::Id;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct ActFrameRef<'a> {
    h: u64,
    p: Hash,
    act: &'a Act,
}

#[derive(Deserialize)]
struct ActFrame {
    h: u64,
    p: Hash,
    act: Act,
}

#[derive(Serialize)]
struct FactFrameRef<'a> {
    h: u64,
    p: Hash,
    fact: &'a Fact,
}

#[derive(Deserialize)]
struct FactFrame {
    h: u64,
    p: Hash,
    fact: Fact,
}

#[derive(Serialize, Deserialize)]
struct ReceivedNote {
    act: Hash,
    ms: u64,
}

/// What the spines held.
#[derive(Debug, Default)]
pub struct Loaded {
    pub acts: Vec<Act>,
    /// Keeper's order.
    pub facts: Vec<Fact>,
    /// Every act id in spine order (across Names, spine by spine).
    pub unsealed_order: Vec<ActId>,
    /// Torn tails skipped.
    pub torn: usize,
}

#[derive(Debug)]
pub struct Spines {
    root: PathBuf,
    /// Per spine file: (height, hash of the last frame).
    heads: RefCell<HashMap<PathBuf, (u64, Hash)>>,
}

impl Spines {
    pub fn new(flit_dir: &Path) -> Self {
        Self { root: flit_dir.to_path_buf(), heads: RefCell::new(HashMap::new()) }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn spines_dir(&self) -> PathBuf {
        self.root.join("spines")
    }

    pub fn sidecar_dir(&self) -> PathBuf {
        self.root.join("sidecar")
    }

    pub fn spine_for(&self, name: &Id) -> PathBuf {
        self.spines_dir().join(format!("{}.tree", name.to_hex()))
    }

    pub fn keeper_path(&self) -> PathBuf {
        self.spines_dir().join("keeper.tree")
    }

    /// Does this folder hold spines?
    pub fn exists(flit_dir: &Path) -> bool {
        flit_dir.join("spines").is_dir()
    }

    /// A song folder from before the spine model (dev-only data; refused).
    pub fn old_layout(flit_dir: &Path) -> bool {
        !Self::exists(flit_dir) && (flit_dir.join("stamps").is_dir() || flit_dir.join("events.jsonl").is_file())
    }

    /// The data format the folder was written with.
    pub fn format_version(flit_dir: &Path) -> crate::Result<u32> {
        let p = flit_dir.join("format");
        if !p.is_file() {
            return Ok(FORMAT_VERSION);
        }
        std::fs::read_to_string(&p)?.trim().parse::<u32>().map_err(|e| crate::Error::Decode(format!("format file: {e}")))
    }

    pub fn ensure_dirs(&self) -> crate::Result<()> {
        if Self::old_layout(&self.root) {
            return Err(crate::Error::Invalid(format!("{} is a song from before the spine model; it cannot be opened", self.root.display())));
        }
        let v = Self::format_version(&self.root)?;
        if v > FORMAT_VERSION {
            return Err(crate::Error::FormatTooNew(v));
        }
        std::fs::create_dir_all(self.spines_dir())?;
        std::fs::create_dir_all(self.sidecar_dir())?;
        let f = self.root.join("format");
        if !f.is_file() {
            std::fs::write(f, format!("{FORMAT_VERSION}\n"))?;
        }
        Ok(())
    }

    // ----- append -----

    fn head_of(&self, path: &Path) -> crate::Result<(u64, Hash)> {
        if let Some(h) = self.heads.borrow().get(path) {
            return Ok(*h);
        }
        let head = if path.is_file() { scan_head(path)? } else { (0, Hash::ZERO) };
        self.heads.borrow_mut().insert(path.to_path_buf(), head);
        Ok(head)
    }

    fn append_line(&self, path: &Path, make: impl FnOnce(u64, Hash) -> String) -> crate::Result<()> {
        let (h, p) = self.head_of(path)?;
        let line = make(h + 1, p);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        f.flush()?;
        self.heads.borrow_mut().insert(path.to_path_buf(), (h + 1, Hash::of(line.as_bytes())));
        Ok(())
    }

    /// Append an act to its Name's spine.
    pub fn append_act(&self, act: &Act) -> crate::Result<()> {
        let path = self.spine_for(&act.name());
        self.append_line(&path, |h, p| serde_json::to_string(&ActFrameRef { h, p, act }).expect("act frame encodes"))
    }

    /// Append a fact to the keeper's spine.
    pub fn append_fact(&self, fact: &Fact) -> crate::Result<()> {
        let path = self.keeper_path();
        self.append_line(&path, |h, p| serde_json::to_string(&FactFrameRef { h, p, fact }).expect("fact frame encodes"))
    }

    // ----- load -----

    /// Read every spine. Frames are checked (height, link, act id).
    pub fn load(&self) -> crate::Result<Loaded> {
        let mut out = Loaded::default();
        let dir = self.spines_dir();
        if !dir.is_dir() {
            return Ok(out);
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|e| e == "tree").unwrap_or(false)).collect();
        files.sort();
        for path in files {
            if path.file_name().map(|n| n == "keeper.tree").unwrap_or(false) {
                let (frames, head, torn, good) = read_frames::<FactFrame>(&path, |f| (f.h, f.p))?;
                self.heads.borrow_mut().insert(path.clone(), head);
                if torn {
                    heal(&path, good)?;
                    out.torn += 1;
                }
                for f in frames {
                    if f.fact.body.hash() != f.fact.id {
                        return Err(crate::Error::Decode(format!("{}: fact frame {} does not match its id", path.display(), f.h)));
                    }
                    out.facts.push(f.fact);
                }
            } else {
                let (frames, head, torn, good) = read_frames::<ActFrame>(&path, |f| (f.h, f.p))?;
                self.heads.borrow_mut().insert(path.clone(), head);
                if torn {
                    heal(&path, good)?;
                    out.torn += 1;
                }
                for f in frames {
                    f.act.check_id().map_err(|e| crate::Error::Decode(format!("{}: frame {}: {e}", path.display(), f.h)))?;
                    out.unsealed_order.push(f.act.id);
                    out.acts.push(f.act);
                }
            }
        }
        Ok(out)
    }

    /// Verify a Name's spine file (copied from another peer) frame by
    /// frame: height, link, id, and that every act is that Name's and
    /// signed by its key. Returns the acts.
    pub fn verify_act_spine(path: &Path, name: &Id) -> crate::Result<Vec<Act>> {
        let (frames, _, _, _) = read_frames::<ActFrame>(path, |f| (f.h, f.p))?;
        let mut acts = Vec::with_capacity(frames.len());
        for f in frames {
            f.act.check_id()?;
            if f.act.name() != *name {
                return Err(crate::Error::Invalid(format!("frame {} belongs to another Name", f.h)));
            }
            if !f.act.signed_by(name) {
                return Err(crate::Error::BadSignature(f.act.id));
            }
            acts.push(f.act);
        }
        Ok(acts)
    }

    // ----- sidecar -----

    fn received_path(&self) -> PathBuf {
        self.sidecar_dir().join("received.jsonl")
    }

    /// Note when this machine first received an act. Best effort.
    pub fn record_received(&self, act: &ActId, ms: u64) {
        let p = self.received_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
            let _ = writeln!(f, "{}", serde_json::to_string(&ReceivedNote { act: *act, ms }).unwrap_or_default());
        }
    }

    pub fn load_received(&self) -> HashMap<ActId, u64> {
        let mut out = HashMap::new();
        if let Ok(text) = std::fs::read_to_string(self.received_path()) {
            for line in text.lines() {
                if let Ok(n) = serde_json::from_str::<ReceivedNote>(line) {
                    out.entry(n.act).or_insert(n.ms);
                }
            }
        }
        out
    }

    /// Throw the sidecar away. Everything in it is rebuilt from the spines.
    pub fn clear_sidecar(&self) -> crate::Result<()> {
        let d = self.sidecar_dir();
        if d.is_dir() {
            std::fs::remove_dir_all(&d)?;
        }
        std::fs::create_dir_all(&d)?;
        Ok(())
    }
}

/// Height and hash of the last complete, linked frame of a spine.
fn scan_head(path: &Path) -> crate::Result<(u64, Hash)> {
    let bytes = std::fs::read(path)?;
    let mut h = 0u64;
    let mut prev = Hash::ZERO;
    for line in complete_lines(&bytes) {
        let v: serde_json::Value = match serde_json::from_slice(line) {
            Ok(v) => v,
            Err(_) => break,
        };
        let height = v.get("h").and_then(|x| x.as_u64()).unwrap_or(0);
        let p = v.get("p").and_then(|x| x.as_str()).and_then(Hash::from_hex).unwrap_or(Hash::ZERO);
        if height != h + 1 || p != prev {
            break;
        }
        h = height;
        prev = Hash::of(line);
    }
    Ok((h, prev))
}

/// Complete lines (ending in a newline) of a file.
fn complete_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
    // the piece after the last newline is either empty or a torn tail
    lines.pop();
    lines.into_iter().map(|l| l.strip_suffix(b"\r").unwrap_or(l)).filter(|l| !l.is_empty())
}

/// Cut a spine back to its last good frame (a crash mid-append heals).
fn heal(path: &Path, good_len: u64) -> crate::Result<()> {
    let f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.set_len(good_len)?;
    Ok(())
}

/// Read a spine's frames. A broken frame in the middle is tampering (an
/// error); a broken last frame is a torn tail (skipped, reported). Also
/// returns the byte length of the good part.
fn read_frames<T: serde::de::DeserializeOwned>(path: &Path, head: impl Fn(&T) -> (u64, Hash)) -> crate::Result<(Vec<T>, (u64, Hash), bool, u64)> {
    let bytes = std::fs::read(path)?;
    let lines: Vec<&[u8]> = complete_lines(&bytes).collect();
    let total = lines.len();
    let mut out = Vec::with_capacity(total);
    let mut h = 0u64;
    let mut prev = Hash::ZERO;
    let mut torn = false;
    let mut good: u64 = 0;
    for (i, line) in lines.into_iter().enumerate() {
        let last = i + 1 == total;
        let frame: T = match serde_json::from_slice(line) {
            Ok(f) => f,
            Err(e) => {
                if last {
                    torn = true;
                    break;
                }
                return Err(crate::Error::Decode(format!("{}: frame {}: {e}", path.display(), i + 1)));
            }
        };
        let (height, p) = head(&frame);
        if height != h + 1 || p != prev {
            if last {
                torn = true;
                break;
            }
            return Err(crate::Error::Decode(format!("{}: frame {} does not link to the one before it", path.display(), i + 1)));
        }
        h = height;
        prev = Hash::of(line);
        out.push(frame);
        // the line plus its newline (and a CR if the file had one)
        let start = line.as_ptr() as usize - bytes.as_ptr() as usize;
        let mut end = start + line.len();
        if bytes.get(end) == Some(&b'\r') {
            end += 1;
        }
        good = (end + 1) as u64;
    }
    // a partial trailing line (no newline) is a torn tail too
    if bytes.last().map(|b| *b != b'\n').unwrap_or(false) {
        torn = true;
    }
    Ok((out, (h, prev), torn, good))
}
