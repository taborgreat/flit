//! Fold tier: replay chains into state.
//!
//! A history's fold order is the keeper's seal order (sealed facts, by
//! seal_seq) followed by the acts nobody sealed yet, in the order they
//! arrived here (optimistic). A history started from a moment folds its
//! parent up to that moment first. Nothing here reads a clock.

use super::Chains;
use crate::act::*;
use crate::state::SongState;
use std::collections::HashSet;

/// The acts of a history in fold order: sealed (seal order) then unsealed.
pub fn sequence<'a>(chains: &'a Chains, history: &HistoryId) -> Vec<&'a Act> {
    let Some(h) = chains.histories.get(history) else { return vec![] };
    let mut v: Vec<&Act> = h.sealed.iter().filter_map(|f| chains.facts.get(f)).filter_map(|f| chains.acts.get(&f.act_id())).collect();
    v.extend(h.unsealed.iter().filter_map(|a| chains.acts.get(a)));
    v
}

/// Position of an act in its history's fold order (sealed: seal_seq - 1;
/// unsealed: after every sealed one, in arrival order).
pub fn position(chains: &Chains, act: &ActId) -> Option<(HistoryId, usize)> {
    if let Some(f) = chains.fact_for(act) {
        return Some((f.history(), f.body.seal_seq as usize - 1));
    }
    let a = chains.acts.get(act)?;
    let h = chains.histories.get(&a.history())?;
    let i = h.unsealed.iter().position(|x| x == act)?;
    Some((a.history(), h.sealed.len() + i))
}

/// Which acts are undone, given a history's acts in fold order. An Undo
/// only counts if it is itself live, so undoing an Undo brings its target
/// back. Undos point backwards, so one reverse pass settles it.
pub fn undone_in<'a>(acts: impl Iterator<Item = &'a Act>) -> HashSet<ActId> {
    let acts: Vec<&Act> = acts.collect();
    let mut undone = HashSet::new();
    for a in acts.into_iter().rev() {
        if undone.contains(&a.id) {
            continue;
        }
        if let Payload::Undo { target } = a.payload() {
            undone.insert(*target);
        }
    }
    undone
}

pub fn undone(chains: &Chains, history: &HistoryId) -> HashSet<ActId> {
    undone_in(sequence(chains, history).into_iter())
}

/// Is `act` in the past of `history`: in it, or in a parent at or before
/// the moment the history forks from?
pub fn in_history(chains: &Chains, history: &HistoryId, act: &ActId) -> bool {
    let Some((act_history, pos)) = position(chains, act) else { return false };
    let mut cur = *history;
    let mut limit: Option<usize> = None;
    loop {
        if act_history == cur {
            return limit.map(|l| pos <= l).unwrap_or(true);
        }
        let Some(h) = chains.histories.get(&cur) else { return false };
        let (Some(parent), Some(from)) = (h.info.parent, h.info.from) else { return false };
        let Some((from_history, from_pos)) = position(chains, &from) else { return false };
        // the fork point may itself be inherited from higher up
        if from_history != parent {
            // walk to where `from` really lives
            cur = from_history;
        } else {
            cur = parent;
        }
        limit = Some(from_pos);
    }
}

/// The folded-in, not-undone acts of `history` up to and including `upto`
/// (inherited parent acts first), in fold order.
pub fn effective<'a>(chains: &'a Chains, history: &HistoryId, upto: Option<&ActId>) -> Vec<&'a Act> {
    let limit = upto.and_then(|u| position(chains, u)).and_then(|(h, p)| if h == *history { Some(p) } else { None });
    let mut out = Vec::new();
    collect_effective(chains, history, limit, &mut out, 0);
    out
}

fn collect_effective<'a>(chains: &'a Chains, history: &HistoryId, limit: Option<usize>, out: &mut Vec<&'a Act>, depth: usize) {
    if depth > 64 {
        return;
    }
    let Some(h) = chains.histories.get(history) else { return };
    if let Some(from) = h.info.from {
        if let Some((from_history, from_pos)) = position(chains, &from) {
            collect_effective(chains, &from_history, Some(from_pos), out, depth + 1);
        }
    }
    let acts = prefix(chains, history, limit);
    let undone = undone_in(acts.iter().copied());
    for a in acts {
        if undone.contains(&a.id) || matches!(a.payload(), Payload::Undo { .. }) {
            continue;
        }
        out.push(a);
    }
}

fn prefix<'a>(chains: &'a Chains, history: &HistoryId, limit: Option<usize>) -> Vec<&'a Act> {
    let seq = sequence(chains, history);
    let n = limit.map(|i| (i + 1).min(seq.len())).unwrap_or(seq.len());
    seq.into_iter().take(n).collect()
}

/// The state a history starts from: its parent's state at the fork
/// (histories), or empty (main).
pub fn base_state(chains: &Chains, history: &HistoryId) -> SongState {
    let Some(h) = chains.histories.get(history) else { return SongState::default() };
    match h.info.from {
        Some(from) => {
            let mut st = state_at(chains, &from).unwrap_or_default();
            st.applied = 0;
            st
        }
        None => SongState::default(),
    }
}

/// Fold a history up to `limit` (an index into its fold order; None = all),
/// on top of its base. `with_unsealed` = false stops at the sealed facts.
/// Starts from the nearest checkpoint at or before the limit. Undo makes
/// the undone set depend on the whole prefix, so a checkpoint is only used
/// when no Undo after it reaches back past it.
pub fn replay(chains: &Chains, history: &HistoryId, limit: Option<usize>, with_unsealed: bool) -> SongState {
    let Some(h) = chains.histories.get(history) else { return SongState::default() };
    let mut acts = prefix(chains, history, limit);
    if !with_unsealed {
        acts.truncate(h.sealed.len());
    }
    let undone = undone_in(acts.iter().copied());
    let n = acts.len();
    let mut start = 0usize;
    let mut st: Option<SongState> = None;
    for (count, snapshot) in h.checkpoints.iter().rev() {
        if *count <= n {
            let reaches_back = acts[*count..].iter().any(|s| matches!(s.payload(), Payload::Undo { target } if acts[..*count].iter().any(|x| x.id == *target)));
            if !reaches_back {
                start = *count;
                st = Some(snapshot.clone());
            }
            break;
        }
    }
    let mut st = st.unwrap_or_else(|| base_state(chains, history));
    for a in &acts[start..] {
        if !undone.contains(&a.id) {
            apply_one(chains, &mut st, a);
        }
    }
    st.applied = n;
    st
}

/// Fold a history's sealed facts from its base and write fresh checkpoints.
pub fn replay_sealed(chains: &Chains, history: &HistoryId) -> (SongState, Vec<(usize, SongState)>) {
    let Some(h) = chains.histories.get(history) else { return (SongState::default(), vec![]) };
    let acts: Vec<&Act> = h.sealed.iter().filter_map(|f| chains.facts.get(f)).filter_map(|f| chains.acts.get(&f.act_id())).collect();
    let undone = undone_in(acts.iter().copied());
    let mut st = base_state(chains, history);
    let mut checkpoints = Vec::new();
    for (i, a) in acts.iter().enumerate() {
        if !undone.contains(&a.id) {
            apply_one(chains, &mut st, a);
        }
        let n = i + 1;
        if n % super::CHECKPOINT_EVERY == 0 {
            let mut snap = st.clone();
            snap.applied = n;
            checkpoints.push((n, snap));
        }
    }
    st.applied = acts.len();
    (st, checkpoints)
}

/// State of the history `at` is in, just after `at` was folded in.
pub fn state_at(chains: &Chains, at: &ActId) -> crate::Result<SongState> {
    let (history, idx) = position(chains, at).ok_or(crate::Error::UnknownAct(*at))?;
    Ok(replay(chains, &history, Some(idx), true))
}

/// Fold one live act into a state. Go back folds the history up to its
/// target and takes every track's sound from there.
pub fn apply_one(chains: &Chains, st: &mut SongState, act: &Act) {
    match act.payload() {
        Payload::Undo { .. } => {}
        Payload::GoBack { to } => {
            if let Ok(past) = state_at(chains, to) {
                st.restore_sound_from(&past);
            }
        }
        _ => st.apply(act),
    }
}
