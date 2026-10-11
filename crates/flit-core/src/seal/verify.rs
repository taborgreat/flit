//! Verify tier: every incoming act and fact, ours or a peer's, before it
//! touches a chain (and so before it can ever reach a host).
//!
//! Acts: id matches body; signature by the Name key or a key the Name
//! certified for that being; act_prev/act_seq continue the being's chain
//! on that history; thing/property/gate match the payload; values in
//! range; the MAY check (crew, being not revoked or blocked, ai beings
//! only where allowed, Crew/Grant/Being/Keeper/Settings/Flag only from the
//! Name key itself).
//!
//! Facts: id matches body; signed by the current keeper (or the previous
//! one while the role moves); fact_prev/fact_seq continue the thing's
//! chain; seal_seq continues the history's seal order; the act is here.
//!
//! Nothing in here reads a clock.

use super::Chains;
use crate::act::*;
use crate::id::Id;

fn bad(msg: impl Into<String>) -> crate::Error {
    crate::Error::Invalid(msg.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActCheck {
    Ok,
    Duplicate,
    /// Waiting for this act (the act_prev).
    Pending(ActId),
    /// Another act already holds this number on the being's chain.
    Equivocation(ActId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactCheck {
    Ok,
    Duplicate,
    /// Waiting for the act it seals.
    Pending(ActId),
    /// Arrived ahead of earlier facts of its history (or of the act that
    /// starts the history); kept until they land.
    Early,
}

/// Does `signer` speak for the act's being in this song?
pub fn signer_ok(chains: &Chains, act: &Act) -> bool {
    let name = act.name();
    if act.signed_by(&name) {
        return true;
    }
    let being = act.being();
    let Some(info) = chains.beings.get(&being) else { return false };
    if info.revoked {
        return false;
    }
    match info.key {
        Some(k) if act.signed_by(&k) => info.scope.is_empty() || chains.song_id().map(|s| info.scope.contains(&s)).unwrap_or(false),
        _ => false,
    }
}

/// Full check of an act whose chain state is known.
pub fn act(chains: &Chains, act: &Act) -> crate::Result<ActCheck> {
    act.check_id()?;
    if act.body.format_version > FORMAT_VERSION {
        return Err(crate::Error::FormatTooNew(act.body.format_version));
    }
    let name = act.name();
    let being = act.being();
    let history = act.history();

    // --- genesis ---
    if chains.genesis.is_none() {
        verify_genesis(act)?;
        if !act.signed_by(&name) {
            return Err(crate::Error::BadSignature(act.id));
        }
        return Ok(ActCheck::Ok);
    }
    if looks_like_genesis(act) {
        return Err(crate::Error::AlreadyHasGenesis);
    }

    // --- signature ---
    if !signer_ok(chains, act) {
        return Err(crate::Error::BadSignature(act.id));
    }

    // --- act chain ---
    let (head_id, head_seq) = chains.act_head(&being, &history).map(|(id, n)| (Some(id), n)).unwrap_or((None, 0));
    if act.body.act_seq == 0 {
        return Err(crate::Error::BadActChain(act.id));
    }
    if act.body.act_seq <= head_seq {
        let existing = chains.act_chains.get(&(being.clone(), history)).and_then(|v| v.get(act.body.act_seq as usize - 1)).copied();
        return Ok(match existing {
            Some(e) if e == act.id => ActCheck::Duplicate,
            Some(e) => ActCheck::Equivocation(e),
            None => return Err(crate::Error::BadActChain(act.id)),
        });
    }
    if act.body.act_seq > head_seq + 1 {
        return Ok(match act.body.act_prev {
            Some(p) if !chains.acts.contains_key(&p) => ActCheck::Pending(p),
            _ => return Err(crate::Error::BadActChain(act.id)),
        });
    }
    if act.body.act_prev != head_id {
        // the prev might be an act we hold as equivocation evidence
        if let Some(p) = act.body.act_prev {
            if !chains.acts.contains_key(&p) && !chains.evidence_acts.contains_key(&p) {
                return Ok(ActCheck::Pending(p));
            }
        }
        return Err(crate::Error::BadActChain(act.id));
    }

    // --- history ---
    if !chains.histories.contains_key(&history) {
        return Err(crate::Error::UnknownHistory(history));
    }

    // --- thing / property / gate follow the payload ---
    check_shape(act)?;
    check_ranges(act)?;

    // --- crew and being ---
    // In the crew; or an invited Name answering an org's invite; or, in a
    // song whose access is "org", a current member of that org.
    if !chains.crew.is_member(&name) {
        let invited_reply = matches!(act.payload(), Payload::Crew(CrewAction::Accept | CrewAction::Decline)) && chains.crew.is_invited(&name);
        let via_org = chains.kind.is_song() && chains.access() == crate::org::Access::Org && chains.org_members.as_ref().map(|m| m.contains(&name)).unwrap_or(false);
        if !invited_reply && !via_org {
            return Err(crate::Error::NotInCrew(name));
        }
    }
    if chains.beings.is_revoked(&being) {
        return Err(crate::Error::RevokedBeing(being));
    }
    if chains.crew.is_blocked(&being) {
        return Err(crate::Error::BlockedBeing(being));
    }
    let is_owner = chains.crew.is_owner(&name);
    let by_name_key = act.signed_by(&name);
    let is_ai = chains.beings.is_ai(&being);

    // --- container kind: songs hold sound; orgs and shelves hold albums ---
    let kind = chains.kind;
    match act.payload() {
        p if !kind.is_song() && (p.is_track_edit() || matches!(p, Payload::Transport(_) | Payload::Version(_) | Payload::Merge(_) | Payload::GoBack { .. } | Payload::Undo { .. })) => {
            return Err(bad("that is a song moment; this is not a song"));
        }
        Payload::Album(_) if kind.is_song() => return Err(bad("albums live in orgs and personal shelves")),
        Payload::Settings(SettingsChange::Org { .. } | SettingsChange::AiPolicy(_)) if !kind.is_song() => return Err(bad("org and ai policy are song settings")),
        Payload::Settings(SettingsChange::Label(_) | SettingsChange::Color(_) | SettingsChange::Defaults(_) | SettingsChange::Archived(_)) if kind.is_song() => return Err(bad("label, color, defaults and archive are org settings")),
        _ => {}
    }

    // --- MAY ---
    match act.payload() {
        Payload::Crew(action) => {
            only_name_key(act, by_name_key, "crew")?;
            if history != main_history() {
                return Err(bad("crew changes live on main"));
            }
            chains.crew.check(&name, action, kind)?;
        }
        Payload::Album(a) => {
            if history != main_history() {
                return Err(bad("albums live on main"));
            }
            if let Thing::Album(id) = act.thing() {
                let album = chains.state(&main_history()).and_then(|s| s.albums.get(id).cloned()).ok_or_else(|| bad("unknown album"))?;
                let creator = album.created_by.as_ref().map(|b| b.name == name).unwrap_or(false);
                if a.is_curation() && !is_owner && !creator {
                    return Err(crate::Error::NotAllowed { being, what: "rename, describe, cover or archive an album it did not create (owners may)".into() });
                }
            }
        }
        Payload::Being(b) => {
            only_name_key(act, by_name_key, "being")?;
            if history != main_history() {
                return Err(bad("beings are declared on main"));
            }
            if matches!(b.action, BeingAction::Update { .. } | BeingAction::Revoke) && chains.beings.get(&BeingRef { name, being: b.being.clone() }).is_none() {
                return Err(crate::Error::UnknownBeing(BeingRef { name, being: b.being.clone() }));
            }
        }
        Payload::Grant(g) => {
            only_name_key(act, by_name_key, "grant")?;
            if g.being.name != name {
                return Err(bad("a Name may only grant its own beings"));
            }
            if history != main_history() {
                return Err(bad("grants live on main"));
            }
        }
        Payload::Flag(f) => {
            only_name_key(act, by_name_key, "flag")?;
            if let FlagKind::Equivocation { acts } = &f.kind {
                if acts[0] == acts[1] {
                    return Err(bad("equivocation evidence needs two different acts"));
                }
            }
        }
        Payload::Keeper(_) | Payload::Settings(_) => {
            only_name_key(act, by_name_key, "settings")?;
            if !is_owner {
                return Err(crate::Error::NotOwner(name));
            }
        }
        Payload::Presence(_) => {
            if *act.thing() != Thing::Presence(being.clone()) {
                return Err(bad("presence lands on the being's own presence thing"));
            }
        }
        Payload::History(h) => {
            if h.parent != history {
                return Err(bad("a history starts from the history it is stamped on"));
            }
            if !chains.in_history(&history, &h.from) {
                return Err(bad("a history must start from a moment in this history's past"));
            }
            if chains.histories.contains_key(&act.id) {
                return Err(bad("history already exists"));
            }
        }
        Payload::GoBack { to } => {
            if !chains.acts.contains_key(to) {
                return Err(crate::Error::UnknownAct(*to));
            }
            if !chains.in_history(&history, to) {
                return Err(bad("can only go back to a moment in this history's past"));
            }
            if is_ai {
                ai_allowed_history(chains, &being, &history)?;
            }
        }
        Payload::Undo { target } => {
            let t = chains.acts.get(target).ok_or(crate::Error::UnknownAct(*target))?;
            if chains.history_of(target) != Some(history) {
                return Err(bad("undo must be on the same history as its target"));
            }
            if matches!(t.kind(), Kind::Crew | Kind::Being | Kind::Grant | Kind::Keeper | Kind::History | Kind::Flag | Kind::Presence | Kind::Settings) {
                return Err(bad(format!("{:?} moments cannot be undone", t.kind())));
            }
            if t.name() != name && !is_owner {
                return Err(crate::Error::NotAllowed { being, what: format!("undo {target:?}") });
            }
            if is_ai {
                ai_allowed_history(chains, &being, &history)?;
            }
        }
        Payload::Merge(m) => {
            if m.from_song.is_none() {
                if !chains.histories.contains_key(&m.from_history) {
                    return Err(crate::Error::UnknownHistory(m.from_history));
                }
                if !chains.acts.contains_key(&m.from_moment) {
                    return Err(crate::Error::UnknownAct(m.from_moment));
                }
            }
            for inner in &m.acts {
                inner.check_id()?;
                if !inner.payload().is_track_edit() {
                    return Err(bad("a merge may only carry track edits"));
                }
                let ok = inner.signed_by(&inner.name()) || chains.beings.certified_keys(&inner.name()).iter().any(|k| inner.signed_by(k)) || m.from_song.is_some();
                if !ok {
                    return Err(crate::Error::BadSignature(inner.id));
                }
            }
            if is_ai {
                let own = ai_owns_history(chains, &being, &history);
                if !own {
                    let tracks: Vec<&TrackId> = m.acts.iter().filter_map(|a| a.track()).collect();
                    if tracks.is_empty() || !tracks.iter().all(|t| chains.beings.granted(&being, t)) {
                        return Err(crate::Error::NotAllowed { being, what: "merge into a history it was not granted".into() });
                    }
                }
            }
        }
        p if p.is_track_edit() => {
            if is_ai {
                let track = act.track().ok_or_else(|| bad("track edit without a track"))?;
                let policy = chains.state(&main_history()).map(|s| s.settings.ai_policy).unwrap_or_default();
                if !ai_owns_history(chains, &being, &history) && (policy == crate::org::AiPolicy::OwnHistoryOnly || !chains.beings.granted(&being, track)) {
                    return Err(crate::Error::NotAllowed { being, what: format!("edit {track} in this history without a grant") });
                }
            }
        }
        Payload::Transport(_) | Payload::Version(_) => {
            if is_ai {
                ai_allowed_history(chains, &being, &history)?;
            }
        }
        Payload::Chat(_) => {}
        _ => {}
    }
    Ok(ActCheck::Ok)
}

fn only_name_key(act: &Act, by_name_key: bool, what: &str) -> crate::Result<()> {
    if by_name_key {
        Ok(())
    } else {
        Err(crate::Error::NotAllowed { being: act.being(), what: format!("stamp {what} acts (only the Name itself may)") })
    }
}

/// An ai being's own history: one it created.
pub fn ai_owns_history(chains: &Chains, being: &BeingRef, history: &HistoryId) -> bool {
    chains.histories.get(history).map(|h| h.info.created_by.as_ref() == Some(being)).unwrap_or(false)
}

fn ai_allowed_history(chains: &Chains, being: &BeingRef, history: &HistoryId) -> crate::Result<()> {
    if ai_owns_history(chains, being, history) {
        Ok(())
    } else {
        Err(crate::Error::NotAllowed { being: being.clone(), what: "change a history it did not create".into() })
    }
}

fn looks_like_genesis(act: &Act) -> bool {
    act.body.act_prev.is_none() && act.body.act_seq == 1 && *act.thing() == Thing::Song && matches!(act.payload(), Payload::Crew(CrewAction::Approve { owner: true, id }) if *id == act.name()) && act.history() == main_history()
}

fn verify_genesis(act: &Act) -> crate::Result<()> {
    if act.body.act_prev.is_some() || act.body.act_seq != 1 || act.history() != main_history() || *act.thing() != Thing::Song {
        return Err(crate::Error::BadGenesis("genesis must be the first act on main's song chain".into()));
    }
    if act.gate() {
        return Err(crate::Error::BadGenesis("genesis is not a gate".into()));
    }
    if crate::org::ContainerKind::from_genesis_property(act.body.property.as_deref()).is_none() {
        return Err(crate::Error::BadGenesis("genesis names an unknown container kind".into()));
    }
    match act.payload() {
        Payload::Crew(CrewAction::Approve { id, owner: true }) if *id == act.name() => Ok(()),
        _ => Err(crate::Error::BadGenesis("genesis must approve its Name as owner".into())),
    }
}

/// Thing level, property and gate flag must follow the payload.
pub fn check_shape(act: &Act) -> crate::Result<()> {
    let p = act.payload();
    let level_ok = match p.thing_level() {
        ThingLevel::Device => matches!(act.thing(), Thing::Device { .. }),
        ThingLevel::Track => matches!(act.thing(), Thing::Track(_)),
        ThingLevel::Settings => *act.thing() == Thing::SongSettings,
        ThingLevel::Presence => matches!(act.thing(), Thing::Presence(_)),
        ThingLevel::Song => *act.thing() == Thing::Song,
        ThingLevel::TrackOrSong => matches!(act.thing(), Thing::Song | Thing::Track(_)),
        ThingLevel::Album => matches!(act.thing(), Thing::Album(_)),
    };
    if !level_ok {
        return Err(bad(format!("{:?} does not belong on {}", act.kind(), act.thing())));
    }
    if act.body.property != p.property() {
        return Err(bad("property does not match the payload"));
    }
    if act.gate() != p.is_gate() {
        return Err(bad("gate flag does not match the act's kind"));
    }
    Ok(())
}

// ----- facts -----

/// Full check of a fact.
pub fn fact(chains: &Chains, fact: &Fact) -> crate::Result<FactCheck> {
    if fact.body.hash() != fact.id {
        return Err(crate::Error::BadId(fact.id));
    }
    let act_id = fact.act_id();
    let Some(act) = chains.acts.get(&act_id) else {
        if chains.evidence_acts.contains_key(&act_id) {
            return Err(bad("the act is equivocation evidence"));
        }
        return Ok(FactCheck::Pending(act_id));
    };
    if let Some(existing) = chains.fact_of.get(&act_id) {
        return if *existing == fact.id { Ok(FactCheck::Duplicate) } else { Err(bad("act sealed twice")) };
    }
    keeper_ok(chains, fact)?;
    let history = fact.history();
    if !chains.histories.contains_key(&history) {
        return Ok(FactCheck::Early);
    }
    if *fact.thing() != *act.thing() {
        return Err(bad("fact thing does not match its act"));
    }
    if history != act.history() && !fact.body.gate {
        return Err(bad("a fact sealed into another history is a gate"));
    }
    if fact.body.gate && history == act.history() {
        return Err(bad("a gate fact lands in another history"));
    }
    // seal order
    let sealed = chains.histories.get(&history).map(|h| h.sealed.len() as u64).unwrap_or(0);
    if fact.body.seal_seq > sealed + 1 {
        return Ok(FactCheck::Early);
    }
    if fact.body.seal_seq != sealed + 1 {
        return Err(crate::Error::BadSealChain(fact.id));
    }
    // thing chain
    let chain = chains.fact_chains.get(&(fact.thing().clone(), history));
    let (prev, len) = chain.map(|v| (v.last().copied(), v.len() as u64)).unwrap_or((None, 0));
    if fact.body.fact_prev != prev || fact.body.fact_seq != len + 1 {
        return Err(crate::Error::BadFactChain(fact.id));
    }
    Ok(FactCheck::Ok)
}

/// The fact must be signed by the current keeper (its Name key or a key
/// the Name certified), or by the previous keeper while the new one has
/// not sealed in that history yet.
fn keeper_ok(chains: &Chains, fact: &Fact) -> crate::Result<()> {
    let mut candidates: Vec<(Id, String)> = Vec::new();
    if let Some(c) = chains.keeper() {
        candidates.push(c);
    }
    let fresh = chains.histories.get(&fact.history()).map(|h| h.keeper_fresh).unwrap_or(false);
    if !fresh {
        if let Some(p) = &chains.keeper.previous {
            candidates.push(p.clone());
        }
    }
    for (name, machine) in candidates {
        if fact.body.keeper != name {
            continue;
        }
        if !machine.is_empty() && machine != fact.body.machine {
            continue;
        }
        let mut signers = vec![name];
        signers.extend(chains.beings.certified_keys(&name));
        return fact.verify_with(&signers);
    }
    Err(crate::Error::WrongKeeper(fact.body.keeper))
}

// ----- ranges -----

/// Limits every copy enforces before applying anything to a host.
pub const MAX_TEXT: usize = 4096;
pub const MAX_HOST_STATE: usize = 4 * 1024 * 1024;
pub const MAX_NOTES: usize = 100_000;
pub const MAX_CHUNKS: usize = 100_000;
pub const MAX_MANIFEST: usize = 10_000;
pub const MAX_MERGE_ACTS: usize = 100_000;
pub const MIN_TEMPO: f64 = 20.0;
pub const MAX_TEMPO: f64 = 999.0;

fn plugin_ok(p: &crate::plugin::PluginIdentity) -> bool {
    !p.name.is_empty() && p.name.len() <= 256 && p.vendor.len() <= 256 && p.version.len() <= 64
}

/// Values in range, sizes bounded, strings sane. Pure; no chains needed.
pub fn check_ranges(act: &Act) -> crate::Result<()> {
    if let Some(t) = act.track() {
        if t.0.is_empty() || t.0.len() > 128 {
            return Err(bad("track id must be 1..128 chars"));
        }
    }
    if act.body.host.0.is_empty() || act.body.host.0.len() > 64 {
        return Err(bad("host tag must be 1..64 chars"));
    }
    if act.body.being.0.is_empty() || act.body.being.0.len() > 64 {
        return Err(bad("being id must be 1..64 chars"));
    }
    if act.body.app_version.len() > 32 {
        return Err(bad("app version too long"));
    }
    if act.body.property.as_ref().map(|p| p.len() > 512).unwrap_or(false) {
        return Err(bad("property too long"));
    }
    if act.body.pos.map(|p| p.ticks < 0).unwrap_or(false) {
        return Err(bad("negative position"));
    }
    match act.payload() {
        Payload::Setting(s) => {
            if !(0.0..=1.0).contains(&s.value) || s.value.is_nan() {
                return Err(bad(format!("setting value {} out of 0..1", s.value)));
            }
            if s.param.0.is_empty() || s.param.0.len() > 256 || !plugin_ok(&s.plugin) {
                return Err(bad("plugin identity or param id too long"));
            }
        }
        Payload::Device(d) => {
            if let Some(p) = &d.plugin {
                if !plugin_ok(p) {
                    return Err(bad("plugin identity out of bounds"));
                }
            }
        }
        Payload::Mixer(m) => match m {
            MixerChange::Volume(v) if !(0.0..=16.0).contains(v) => return Err(bad(format!("volume {v} out of 0..16"))),
            MixerChange::Pan(p) if !(-1.0..=1.0).contains(p) => return Err(bad(format!("pan {p} out of -1..1"))),
            _ => {}
        },
        Payload::Transport(t) => match t {
            TransportChange::Tempo(b) if !(MIN_TEMPO..=MAX_TEMPO).contains(b) => return Err(bad(format!("tempo {b} out of range"))),
            TransportChange::Position(p) if p.ticks < 0 => return Err(bad("negative position")),
            TransportChange::Loop { start, end } if start.ticks < 0 || end.ticks <= start.ticks => return Err(bad("bad loop range")),
            _ => {}
        },
        Payload::Notes(n) => {
            if n.clip.is_empty() || n.clip.len() > 256 || n.add.len() + n.remove.len() > MAX_NOTES || n.add.is_empty() && n.remove.is_empty() {
                return Err(bad("notes payload out of bounds"));
            }
            if n.add.iter().chain(n.remove.iter()).any(|x| x.key > 127 || x.velocity > 127 || x.channel > 15 || x.length_ticks <= 0 || x.start.ticks < 0) {
                return Err(bad("note out of range"));
            }
        }
        Payload::Audio(a) => {
            if !(8000..=384_000).contains(&a.sample_rate) || !matches!(a.bit_depth, 16 | 24 | 32) || a.channels == 0 || a.channels > 2 {
                return Err(bad("audio format out of range"));
            }
            if a.opus_chunks.len() > MAX_CHUNKS || a.flac_chunks.len() > MAX_CHUNKS || a.start.ticks < 0 {
                return Err(bad("audio payload out of bounds"));
            }
            if !a.flac_chunks.is_empty() && !a.opus_chunks.is_empty() && a.flac_chunks.len() != a.opus_chunks.len() {
                return Err(bad("opus and flac chunk sets must share bar boundaries"));
            }
        }
        Payload::Version(v) => {
            if v.name.len() > MAX_TEXT || v.manifest.len() > MAX_MANIFEST || v.manifest.iter().any(|m| m.path.len() > 4096) {
                return Err(bad("version payload out of bounds"));
            }
        }
        Payload::Being(b) => match &b.action {
            BeingAction::Create { display_name, scope, .. } => {
                if display_name.len() > 64 || display_name.chars().any(|c| c.is_control()) || scope.len() > 1024 {
                    return Err(bad("being out of bounds"));
                }
            }
            BeingAction::Update { display_name, .. } => {
                if display_name.len() > 64 || display_name.chars().any(|c| c.is_control()) {
                    return Err(bad("display name out of bounds"));
                }
            }
            BeingAction::Revoke => {}
        },
        Payload::Grant(g) => {
            if g.tracks.is_empty() || g.tracks.len() > 1024 || g.tracks.iter().any(|t| t.0.is_empty() || t.0.len() > 128) {
                return Err(bad("grant out of bounds"));
            }
        }
        Payload::Flag(f) => {
            if f.note.len() > MAX_TEXT {
                return Err(bad("flag note too long"));
            }
        }
        Payload::Keeper(k) => {
            if k.machine.len() > 128 {
                return Err(bad("machine id too long"));
            }
        }
        Payload::History(h) => {
            if h.name.is_empty() || h.name.len() > 256 {
                return Err(bad("history name must be 1..256 chars"));
            }
        }
        Payload::Merge(m) => {
            if m.acts.len() > MAX_MERGE_ACTS || m.tracks.len() > 4096 || m.choices.len() > MAX_MERGE_ACTS {
                return Err(bad("merge too large"));
            }
            for inner in &m.acts {
                check_ranges(inner)?;
            }
        }
        Payload::Chat(c) => {
            if c.text.len() > MAX_TEXT {
                return Err(bad("chat text too long"));
            }
        }
        Payload::Presence(p) => {
            if p.region.as_ref().map(|r| r.len() > 256).unwrap_or(false) {
                return Err(bad("presence region too long"));
            }
        }
        Payload::HostState(h) => {
            if h.blob.len() > MAX_HOST_STATE || h.host.0.is_empty() {
                return Err(bad("host state too large"));
            }
        }
        Payload::Settings(s) => match s {
            SettingsChange::RequiredVersion(v) => {
                if v.is_empty() || v.len() > 32 {
                    return Err(bad("required version must be 1..32 chars"));
                }
            }
            SettingsChange::TimeSignature { beats, unit } => {
                if *beats == 0 || *beats > 32 || !matches!(unit, 1 | 2 | 4 | 8 | 16 | 32) {
                    return Err(bad("time signature out of range"));
                }
            }
            SettingsChange::ReferenceDaw(_) => {}
            SettingsChange::Label(l) => {
                if l.len() > 128 || l.chars().any(|c| c.is_control()) {
                    return Err(bad("label out of bounds"));
                }
            }
            SettingsChange::Defaults(d) => {
                if d.required_version.as_ref().map(|v| v.is_empty() || v.len() > 32).unwrap_or(false) || d.keeper.as_ref().map(|(_, m)| m.len() > 128).unwrap_or(false) {
                    return Err(bad("org defaults out of bounds"));
                }
            }
            SettingsChange::Org { .. } | SettingsChange::AiPolicy(_) | SettingsChange::Color(_) | SettingsChange::Archived(_) => {}
        },
        Payload::Album(a) => match a {
            crate::org::AlbumAction::Create { name } | crate::org::AlbumAction::Rename(name) => {
                if name.is_empty() || name.len() > 128 || name.chars().any(|c| c.is_control()) {
                    return Err(bad("album name must be 1..128 chars"));
                }
            }
            crate::org::AlbumAction::SetDescription(d) => {
                if d.len() > MAX_TEXT {
                    return Err(bad("album description too long"));
                }
            }
            crate::org::AlbumAction::Reorder(order) => {
                if order.len() > 10_000 {
                    return Err(bad("album order too long"));
                }
            }
            crate::org::AlbumAction::SetCover(_) | crate::org::AlbumAction::AddSong { .. } | crate::org::AlbumAction::RemoveSong { .. } | crate::org::AlbumAction::Archive(_) => {}
        },
        Payload::Crew(_) | Payload::GoBack { .. } | Payload::Undo { .. } => {}
    }
    Ok(())
}
