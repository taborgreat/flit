//! Lenses: timeline views over the chains (update pack §7). Each lens
//! reads its own chain. Lenses change what you see and its order; never
//! the fold. Order within a lens comes from chain numbers only (seal_seq,
//! fact_seq, act_seq); unsealed acts come last, in arrival order. Rows
//! carry the two witness times for display, and nothing here sorts by them.

use super::Chains;
use crate::act::*;
use crate::id::Id;
use crate::plugin::DeviceSlot;
use crate::position::SongPos;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lens {
    /// All facts in the history, keeper seal order.
    Song,
    SongSettings,
    /// That track and all its devices, fact order.
    Track(TrackId),
    Device { track: TrackId, slot: DeviceSlot },
    /// The current being's act chain, act order.
    Me(BeingRef),
    /// All beings of a Name, by the seal order of their facts.
    Name(Id),
    /// One being's act chain, act order.
    Being(BeingRef),
    /// All crew beings, seal order (same as Song).
    Team,
    /// Both filters, e.g. Me x Track Drums.
    Combine(Box<Lens>, Box<Lens>),
}

impl Lens {
    pub fn is_being_lens(&self) -> bool {
        match self {
            Lens::Me(_) | Lens::Being(_) => true,
            Lens::Combine(a, b) => a.is_being_lens() || b.is_being_lens(),
            _ => false,
        }
    }

    pub fn is_thing_lens(&self) -> bool {
        match self {
            Lens::SongSettings | Lens::Track(_) | Lens::Device { .. } => true,
            Lens::Combine(a, b) => a.is_thing_lens() || b.is_thing_lens(),
            _ => false,
        }
    }

    fn keeps(&self, row: &Row) -> bool {
        match self {
            Lens::Song | Lens::Team => true,
            Lens::SongSettings => row.thing == Thing::SongSettings,
            Lens::Track(t) => row.thing.track() == Some(t),
            Lens::Device { track, slot } => matches!(&row.thing, Thing::Device { track: t, slot: s } if t == track && s == slot),
            Lens::Me(b) | Lens::Being(b) => row.being == *b,
            Lens::Name(n) => row.being.name == *n,
            Lens::Combine(a, b) => a.keeps(row) && b.keeps(row),
        }
    }
}

/// One mark on the tape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub act: ActId,
    pub fact: Option<FactId>,
    /// The hover-card number: fact_seq on thing lenses, act_seq on being
    /// lenses, seal_seq otherwise (0 while unsealed).
    pub number: u64,
    pub act_seq: u64,
    pub fact_seq: Option<u64>,
    pub seal_seq: Option<u64>,
    /// The history the row counts in.
    pub history: HistoryId,
    /// From a parent history, before the fork.
    pub inherited: bool,
    pub being: BeingRef,
    pub kind: Kind,
    pub thing: Thing,
    pub property: Option<String>,
    pub summary: String,
    pub gate: bool,
    pub sealed: bool,
    pub undone: bool,
    /// Acts a Merge landed as one group (1 for ordinary rows).
    pub group_size: usize,
    /// Witness only.
    pub stamped_at: u64,
    pub received_at: Option<u64>,
    pub app_version: String,
    pub host: HostTag,
    pub pos: Option<SongPos>,
}

/// The whole tape of a history: inherited parent moments up to the fork,
/// then the history's own, in fold order. Each entry tagged with its
/// segment (0 = oldest ancestor).
fn tape<'a>(chains: &'a Chains, history: &HistoryId) -> Vec<(&'a Act, usize, bool)> {
    let mut segments: Vec<(HistoryId, Option<usize>)> = Vec::new();
    let mut cur = *history;
    let mut limit: Option<usize> = None;
    let mut guard = 0;
    loop {
        segments.push((cur, limit));
        let Some(h) = chains.histories.get(&cur) else { break };
        let Some(from) = h.info.from else { break };
        let Some((from_history, from_pos)) = chains.position(&from) else { break };
        cur = from_history;
        limit = Some(from_pos);
        guard += 1;
        if guard > 64 {
            break;
        }
    }
    segments.reverse();
    let mut out = Vec::new();
    let last = segments.len() - 1;
    for (i, (h, limit)) in segments.into_iter().enumerate() {
        let seq = chains.sequence(&h);
        let n = limit.map(|l| (l + 1).min(seq.len())).unwrap_or(seq.len());
        for a in seq.into_iter().take(n) {
            out.push((a, i, i != last));
        }
    }
    out
}

/// Rows of a history through a lens. Presence rows are hidden unless asked.
pub fn rows(chains: &Chains, history: &HistoryId, lens: &Lens, show_presence: bool) -> Vec<Row> {
    let tape = tape(chains, history);
    // undone sets per segment history
    let mut undone_by_history: Vec<(HistoryId, HashSet<ActId>)> = Vec::new();
    let mut rows: Vec<(Row, usize)> = Vec::new();
    for (act, segment, inherited) in tape {
        let hist = chains.history_of(&act.id).unwrap_or(act.history());
        let undone = match undone_by_history.iter().find(|(h, _)| *h == hist) {
            Some((_, u)) => u.clone(),
            None => {
                let u = chains.undone(&hist);
                undone_by_history.push((hist, u.clone()));
                u
            }
        };
        if !show_presence && act.kind() == Kind::Presence {
            continue;
        }
        let fact = chains.fact_for(&act.id);
        let row = Row {
            act: act.id,
            fact: fact.map(|f| f.id),
            number: 0,
            act_seq: act.body.act_seq,
            fact_seq: fact.map(|f| f.body.fact_seq),
            seal_seq: fact.map(|f| f.body.seal_seq),
            history: hist,
            inherited,
            being: act.being(),
            kind: act.kind(),
            thing: act.thing().clone(),
            property: act.body.property.clone(),
            summary: summarize(act, chains),
            gate: act.gate() || fact.map(|f| f.body.gate).unwrap_or(false),
            sealed: fact.is_some(),
            undone: undone.contains(&act.id),
            group_size: match act.payload() {
                Payload::Merge(m) => m.acts.len().max(1),
                _ => 1,
            },
            stamped_at: act.body.stamped_at,
            received_at: chains.received_at(&act.id),
            app_version: act.body.app_version.clone(),
            host: act.body.host.clone(),
            pos: act.pos(),
        };
        if lens.keeps(&row) {
            rows.push((row, segment));
        }
    }
    if lens.is_being_lens() {
        // act order within each segment (unsealed acts included: their
        // act_seq is fixed at stamping)
        rows.sort_by_key(|(r, seg)| (*seg, r.act_seq));
    }
    let mut out: Vec<Row> = rows.into_iter().map(|(r, _)| r).collect();
    for r in &mut out {
        r.number = if lens.is_being_lens() {
            r.act_seq
        } else if lens.is_thing_lens() {
            r.fact_seq.unwrap_or(0)
        } else {
            r.seal_seq.unwrap_or(0)
        };
    }
    out
}

/// What property an act changed, for people ("Comp 7", "volume", "slot 2").
pub fn property_label(act: &Act) -> String {
    match act.payload() {
        Payload::Setting(s) => format!("{} {}", s.plugin.name, s.param),
        Payload::Mixer(m) => m.property().to_string(),
        Payload::Device(d) => format!("slot {}", d.slot.0 + 1),
        Payload::Notes(n) => format!("clip {}", n.clip),
        Payload::Audio(a) => format!("audio at {}", a.start.display(Default::default())),
        Payload::Transport(TransportChange::Tempo(_)) => "tempo".into(),
        Payload::Settings(SettingsChange::ReferenceDaw(_)) => "reference DAW".into(),
        Payload::Settings(SettingsChange::RequiredVersion(_)) => "required version".into(),
        Payload::Settings(SettingsChange::TimeSignature { .. }) => "time signature".into(),
        Payload::Keeper(_) => "keeper".into(),
        _ => act.body.property.clone().unwrap_or_else(|| format!("{:?}", act.kind()).to_lowercase()),
    }
}

/// One line for a row.
pub fn summarize(act: &Act, chains: &Chains) -> String {
    let who = |id: &Id| chains.beings.of_name(id).first().map(|(_, b)| b.display_name.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| id.short_code());
    let being_label = |b: &BeingRef| chains.beings.display_name(b).map(str::to_string).unwrap_or_else(|| b.being.0.clone());
    match act.payload() {
        Payload::Setting(s) => format!("{} {} = {:.3}", s.plugin.name, s.param, s.value),
        Payload::Device(d) => match &d.plugin {
            Some(p) => format!("{} added in slot {}", p.name, d.slot.0 + 1),
            None => format!("slot {} emptied", d.slot.0 + 1),
        },
        Payload::Mixer(m) => match m {
            MixerChange::Volume(v) => format!("volume {v:.3}"),
            MixerChange::Pan(p) => format!("pan {p:.2}"),
            MixerChange::Mute(b) => if *b { "mute".into() } else { "unmute".into() },
        },
        Payload::Transport(t) => match t {
            TransportChange::Play => "play".into(),
            TransportChange::Stop => "stop".into(),
            TransportChange::Position(p) => format!("position {}", p.display(Default::default())),
            TransportChange::Tempo(b) => format!("tempo {b:.2}"),
            TransportChange::Loop { start, end } => format!("loop {}-{}", start.display(Default::default()), end.display(Default::default())),
            TransportChange::LoopOff => "loop off".into(),
        },
        Payload::Notes(n) => format!("notes in {} (+{} -{})", n.clip, n.add.len(), n.remove.len()),
        Payload::Audio(a) => format!("audio {} bars", a.bars()),
        Payload::Version(v) => format!("Save version \"{}\" ({})", v.name, v.host),
        Payload::Crew(c) => match c {
            CrewAction::Approve { id, owner } => format!("approved {}{}", who(id), if *owner { " as owner" } else { "" }),
            CrewAction::Remove { id } => format!("removed {}", who(id)),
            CrewAction::Promote { id } => format!("promoted {} to owner", who(id)),
            CrewAction::BlockBeing { being } => format!("blocked {}", being_label(being)),
            CrewAction::UnblockBeing { being } => format!("unblocked {}", being_label(being)),
            CrewAction::Pardon { being } => format!("accepted {}'s moments again", being_label(being)),
        },
        Payload::Being(b) => match &b.action {
            BeingAction::Create { kind, display_name, .. } => format!("being \"{display_name}\" ({kind})"),
            BeingAction::Update { display_name, .. } => format!("being {} is now \"{display_name}\"", b.being),
            BeingAction::Revoke => format!("being {} revoked", b.being),
        },
        Payload::Grant(g) => format!("{} {} on {}", if g.grant { "granted" } else { "revoked" }, being_label(&g.being), g.tracks.iter().map(|t| t.0.as_str()).collect::<Vec<_>>().join(", ")),
        Payload::Flag(f) => match &f.kind {
            FlagKind::ClaimedKind => format!("flagged {}: {}", being_label(&f.being), f.note),
            FlagKind::Equivocation { .. } => format!("{} stamped two moments with one number", being_label(&f.being)),
        },
        Payload::Keeper(k) => format!("keeper is now {} on {}", who(&k.name), k.machine),
        Payload::History(h) => format!("New history \"{}\"", h.name),
        Payload::Merge(m) => format!(
            "Merge {} changes from {}",
            m.acts.len(),
            if m.from_song.is_some() { "another song".to_string() } else { chains.history_name(&m.from_history).map(str::to_string).unwrap_or_else(|| "a history".into()) }
        ),
        Payload::Chat(c) => format!("\"{}\"", c.text),
        Payload::Presence(p) => match (&p.track, p.device) {
            (Some(t), Some(d)) => format!("looking at {t} slot {}", d.0 + 1),
            (Some(t), None) => format!("looking at {t}"),
            _ => format!("in {:?}", p.view).to_lowercase(),
        },
        Payload::HostState(h) => format!("{} state ({} bytes)", h.host, h.blob.len()),
        Payload::GoBack { to } => format!("Go back to {to:?}"),
        Payload::Undo { target } => format!("Undo {target:?}"),
        Payload::Settings(s) => match s {
            SettingsChange::ReferenceDaw(b) => format!("reference DAW: {}", being_label(b)),
            SettingsChange::RequiredVersion(v) => format!("requires FLit {v}"),
            SettingsChange::TimeSignature { beats, unit } => format!("time signature {beats}/{unit}"),
        },
    }
}
