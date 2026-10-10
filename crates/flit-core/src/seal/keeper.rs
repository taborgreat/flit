//! Keeper tier: the one machine per song that seals facts (update pack §4).
//!
//! For every unsealed act, in arrival order: seal it into its history, or
//! decline it into "<Being>'s version of <property>" when its based_on is
//! stale AND the same property was sealed since (by someone else), or hold
//! it (wrong app version, being under equivocation hold). Nothing dropped;
//! nothing here reads a clock.

use super::{AppendOutcome, Chains};
use crate::act::*;
use crate::id::{Id, Keypair};

/// What this machine needs to seal: the key that speaks for the keeper
/// Name (its own key, or a certified being key) and who it acts as when
/// it has to stamp a History act for a decline.
#[derive(Clone, Debug)]
pub struct KeeperKey {
    pub key: Keypair,
    pub name: Id,
    pub being: BeingId,
    pub machine: String,
    pub host: HostTag,
    pub app_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldReason {
    /// The act was made by another FLit version than the song requires.
    Version { required: String, got: String },
    /// The being stamped two acts with one number; an owner must decide.
    Equivocation(BeingRef),
    /// This machine does not hold the role.
    NotKeeper,
}

#[derive(Debug, Clone)]
pub enum Decision {
    Sealed(Fact),
    /// Sealed into another history as a gate.
    Declined {
        fact: Fact,
        history: HistoryId,
        /// The History act (and its fact) stamped to make room, if the
        /// history did not exist yet.
        created: Option<(Act, Fact)>,
    },
    Held { act: ActId, reason: HoldReason },
}

impl Decision {
    pub fn fact(&self) -> Option<&Fact> {
        match self {
            Decision::Sealed(f) | Decision::Declined { fact: f, .. } => Some(f),
            Decision::Held { .. } => None,
        }
    }
}

/// Does this key hold the keeper role right now?
pub fn is_keeper(chains: &Chains, k: &KeeperKey) -> bool {
    match chains.keeper() {
        Some((name, machine)) => name == k.name && (machine.is_empty() || machine == k.machine),
        None => false,
    }
}

/// Seal every unsealed act of every history, in arrival order. Held acts
/// stay unsealed (and applied optimistically); everything else lands.
pub fn seal_pending(chains: &mut Chains, k: &KeeperKey, now_ms: u64) -> crate::Result<Vec<Decision>> {
    let mut out = Vec::new();
    if !is_keeper(chains, k) {
        return Ok(out);
    }
    let mut histories: Vec<HistoryId> = chains.histories.keys().copied().collect();
    histories.sort_by_key(|h| (*h != main_history(), *h));
    for h in histories {
        let pending: Vec<ActId> = chains.histories.get(&h).map(|x| x.unsealed.clone()).unwrap_or_default();
        for id in pending {
            if chains.is_sealed(&id) {
                continue;
            }
            out.push(seal_one(chains, k, &id, now_ms)?);
        }
    }
    Ok(out)
}

/// Seal (or decline, or hold) one act.
pub fn seal_one(chains: &mut Chains, k: &KeeperKey, act_id: &ActId, now_ms: u64) -> crate::Result<Decision> {
    let act = chains.get(act_id).cloned().ok_or(crate::Error::UnknownAct(*act_id))?;
    if chains.is_sealed(act_id) {
        return Err(crate::Error::Duplicate(*act_id));
    }
    if !is_keeper(chains, k) {
        return Ok(Decision::Held { act: *act_id, reason: HoldReason::NotKeeper });
    }
    let being = act.being();
    if chains.held.contains(&being) {
        return Ok(Decision::Held { act: *act_id, reason: HoldReason::Equivocation(being) });
    }
    // the act that sets the required version is the one exception
    let sets_version = matches!(act.payload(), Payload::Settings(SettingsChange::RequiredVersion(_)));
    if let (Some(required), false) = (chains.required_version(), sets_version) {
        if act.body.app_version != required {
            // held until the being updates; once a later act of theirs on
            // this history carries the required version, the old one is
            // sealed aside as a gate (kept, never applied to the history)
            let updated = chains.act_chain(&being, &act.history()).into_iter().skip(act.body.act_seq as usize).any(|a| a.body.app_version == required);
            if !updated {
                return Ok(Decision::Held { act: *act_id, reason: HoldReason::Version { required, got: act.body.app_version.clone() } });
            }
            let label = chains.beings.display_name(&being).map(str::to_string).unwrap_or_else(|| being.being.0.clone());
            let name = format!("{label} on FLit {}", act.body.app_version);
            return decline(chains, k, &act, name, now_ms);
        }
    }

    // same property sealed since based_on by someone else?
    let collides = match (&act.body.property, act.thing()) {
        (Some(prop), thing) if !matches!(thing, Thing::Presence(_)) => {
            let based = act.body.based_on.unwrap_or(0) as usize;
            chains
                .fact_chain(thing, &act.history())
                .into_iter()
                .skip(based)
                .filter_map(|f| chains.get(&f.act_id()))
                .any(|a| a.being() != being && a.body.property.as_deref() == Some(prop.as_str()))
        }
        _ => false,
    };

    if !collides {
        let fact = build_fact(chains, k, &act, act.history(), false);
        land(chains, fact.clone())?;
        return Ok(Decision::Sealed(fact));
    }

    // decline into "<Being>'s version of <property>"
    let label = chains.beings.display_name(&being).map(str::to_string).unwrap_or_else(|| being.being.0.clone());
    let prop_label = super::lens::property_label(&act);
    let name = format!("{label}'s version of {prop_label}");
    decline(chains, k, &act, name, now_ms)
}

/// Seal an act into the history called `name` off the act's history
/// (creating it from the moment the being last saw), as a gate.
fn decline(chains: &mut Chains, k: &KeeperKey, act: &Act, name: String, now_ms: u64) -> crate::Result<Decision> {
    let parent = act.history();
    let existing = chains.histories.values().find(|h| h.info.parent == Some(parent) && h.info.name == name && h.info.created_by.as_ref().map(|b| b.name == k.name).unwrap_or(false)).map(|h| h.info.id);
    let (history, created) = match existing {
        Some(h) => (h, None),
        None => {
            let from = decline_from(chains, act);
            let keeper_being = BeingRef { name: k.name, being: k.being.clone() };
            let (prev, seq) = chains.act_head(&keeper_being, &parent).map(|(id, n)| (Some(id), n + 1)).unwrap_or((None, 1));
            let payload = Payload::History(HistoryPayload { name: name.clone(), from, parent });
            let body = ActBody {
                name: k.name,
                being: k.being.clone(),
                act_prev: prev,
                act_seq: seq,
                thing: Thing::Song,
                property: None,
                based_on: None,
                history: parent,
                app_version: k.app_version.clone(),
                format_version: FORMAT_VERSION,
                host: k.host.clone(),
                gate: false,
                payload,
                pos: None,
                stamped_at: now_ms,
            };
            let hist_act = Act::sign(body, &k.key);
            match chains.append_act(hist_act.clone(), Some(now_ms))? {
                AppendOutcome::Appended => {}
                other => return Err(crate::Error::Invalid(format!("keeper's history act not appended: {other:?}"))),
            }
            let hist_fact = build_fact(chains, k, &hist_act, parent, false);
            land(chains, hist_fact.clone())?;
            (hist_act.id, Some((hist_act, hist_fact)))
        }
    };
    let fact = build_fact(chains, k, act, history, true);
    land(chains, fact.clone())?;
    Ok(Decision::Declined { fact, history, created })
}

/// Where a declined act's history forks from: the moment its being last
/// saw (the fact at based_on on that thing), else where its history starts.
fn decline_from(chains: &Chains, act: &Act) -> ActId {
    let based = act.body.based_on.unwrap_or(0) as usize;
    if based > 0 {
        if let Some(f) = chains.fact_chain(act.thing(), &act.history()).get(based - 1) {
            return f.act_id();
        }
    }
    chains.histories.get(&act.history()).and_then(|h| h.info.from).or(chains.genesis).expect("a song has a genesis")
}

fn build_fact(chains: &Chains, k: &KeeperKey, act: &Act, history: HistoryId, gate: bool) -> Fact {
    let chain = chains.fact_chains.get(&(act.thing().clone(), history));
    let (prev, len) = chain.map(|v| (v.last().copied(), v.len() as u64)).unwrap_or((None, 0));
    let seal_seq = chains.seal_seq(&history) + 1;
    let body = FactBody { thing: act.thing().clone(), history, fact_seq: len + 1, fact_prev: prev, seal_seq, act_id: act.id, keeper: k.name, machine: k.machine.clone(), gate };
    let id = body.hash();
    let keeper_sig = k.key.sign(id.as_bytes());
    Fact { id, keeper_sig, body }
}

fn land(chains: &mut Chains, fact: Fact) -> crate::Result<()> {
    match chains.append_fact(fact)? {
        AppendOutcome::Appended => Ok(()),
        other => Err(crate::Error::Invalid(format!("keeper's own fact not appended: {other:?}"))),
    }
}
