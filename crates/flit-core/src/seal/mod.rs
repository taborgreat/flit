//! The seal: act chains, fact chains and the four tiers that run them
//! (update pack §3). Hash -> verify -> fold -> stamp/seal.
//!
//! - `verify`: every incoming act (signature by the Name key or a certified
//!   being key, act_prev/act_seq continue the being's chain, MAY check) and
//!   every incoming fact (keeper signature, fact_prev/fact_seq continue the
//!   thing's chain, seal_seq continues the history's seal order).
//! - `fold`:   replay sealed facts into state from the nearest checkpoint;
//!   unsealed acts applied on top optimistically.
//! - `keeper`: seal acts into facts, decline same-property collisions into
//!   a history, hold acts from the wrong app version.
//! - `spine`:  the on-disk truth (one append-only spine per Name plus the
//!   keeper's spine); everything else is a rebuildable sidecar.
//! - `lens`:   timeline views over the chains. Never change the fold.
//!
//! `Chains` is the in-memory store behind all of it. The rest of the crate
//! (and every other crate) only talks to `Chains` and `Song`, so this
//! module can be swapped for the TreeOS factum crate later. Nothing in
//! here reads a clock: order comes only from chains.

pub mod fold;
pub mod keeper;
pub mod lens;
pub mod spine;
pub mod verify;

use crate::act::*;
use crate::beings::{Beings, FlagRecord};
use crate::blob::Hash;
use crate::crew::CrewState;
use crate::id::Id;
use crate::state::SongState;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

/// A fold checkpoint every this many sealed facts of a history.
pub const CHECKPOINT_EVERY: usize = 500;

/// What a history is (what branches were).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryInfo {
    pub id: HistoryId,
    pub name: String,
    /// The history it was started from (None for main).
    pub parent: Option<HistoryId>,
    /// The moment it starts from (an act in the parent's past).
    pub from: Option<ActId>,
    pub created_by: Option<BeingRef>,
}

impl HistoryInfo {
    pub fn is_main(&self) -> bool {
        self.id == main_history()
    }
}

/// One history's chains and fold.
#[derive(Debug, Clone)]
pub(crate) struct HistoryChains {
    pub info: HistoryInfo,
    /// Sealed facts in the keeper's seal order (index = seal_seq - 1).
    pub sealed: Vec<FactId>,
    /// Acts applied optimistically, in arrival order, waiting for a fact.
    pub unsealed: Vec<ActId>,
    /// Fold of `sealed` only.
    pub sealed_state: SongState,
    /// Fold of `sealed` + `unsealed`: what the user sees and hears.
    pub view: SongState,
    /// (sealed count, state) every `CHECKPOINT_EVERY` facts.
    pub checkpoints: Vec<(usize, SongState)>,
    /// The current keeper has sealed here since the role last moved.
    pub keeper_fresh: bool,
}

impl HistoryChains {
    fn new(info: HistoryInfo, base: SongState) -> Self {
        Self { info, sealed: Vec::new(), unsealed: Vec::new(), sealed_state: base.clone(), view: base, checkpoints: Vec::new(), keeper_fresh: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOutcome {
    Appended,
    Duplicate,
    /// A prev (or the act a fact seals) is not here yet; kept aside.
    Pending,
    /// A second act with a number already used by that being: kept as
    /// evidence, never applied.
    Equivocation,
}

/// Proof of cheating: two validly signed acts by one being with the same
/// act number on one history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Equivocation {
    pub being: BeingRef,
    pub history: HistoryId,
    pub act_seq: u64,
    pub acts: [ActId; 2],
}

#[derive(Debug, Clone, Default)]
pub(crate) struct KeeperState {
    pub current: Option<(Id, String)>,
    pub previous: Option<(Id, String)>,
}

/// Every act and fact of a song, indexed by chain, plus the folded state
/// of every history. Append-only.
#[derive(Debug)]
pub struct Chains {
    pub(crate) acts: HashMap<ActId, Act>,
    pub(crate) facts: HashMap<FactId, Fact>,
    pub(crate) fact_of: HashMap<ActId, FactId>,
    /// Act chains: (being, history) -> acts in chain order (index = act_seq - 1).
    pub(crate) act_chains: HashMap<(BeingRef, HistoryId), Vec<ActId>>,
    /// Fact chains: (thing, history) -> facts in chain order (index = fact_seq - 1).
    pub(crate) fact_chains: HashMap<(Thing, HistoryId), Vec<FactId>>,
    pub(crate) histories: HashMap<HistoryId, HistoryChains>,
    pub(crate) genesis: Option<ActId>,
    pub(crate) crew: CrewState,
    pub(crate) beings: Beings,
    pub(crate) keeper: KeeperState,
    /// Acts waiting for their act_prev.
    pub(crate) pending_acts: HashMap<ActId, Vec<Act>>,
    /// Facts waiting for their act.
    pub(crate) pending_facts: HashMap<ActId, Vec<Fact>>,
    pub(crate) evidence: Vec<Equivocation>,
    pub(crate) evidence_acts: HashMap<ActId, Act>,
    /// Beings whose new acts the keeper holds until an owner decides.
    pub(crate) held: BTreeSet<BeingRef>,
    /// When THIS machine first received each act (sidecar; display only).
    pub(crate) received_at: HashMap<ActId, u64>,
    pub(crate) store: Option<spine::Spines>,
}

impl Chains {
    pub fn in_memory() -> Self {
        let mut histories = HashMap::new();
        let main = HistoryInfo { id: main_history(), name: "main".into(), parent: None, from: None, created_by: None };
        histories.insert(main_history(), HistoryChains::new(main, SongState::default()));
        Self {
            acts: HashMap::new(),
            facts: HashMap::new(),
            fact_of: HashMap::new(),
            act_chains: HashMap::new(),
            fact_chains: HashMap::new(),
            histories,
            genesis: None,
            crew: CrewState::default(),
            beings: Beings::default(),
            keeper: KeeperState::default(),
            pending_acts: HashMap::new(),
            pending_facts: HashMap::new(),
            evidence: Vec::new(),
            evidence_acts: HashMap::new(),
            held: BTreeSet::new(),
            received_at: HashMap::new(),
            store: None,
        }
    }

    /// Open (or create) the chains under a `.flit` folder by replaying its
    /// spines. The keeper's spine gives the order; acts nobody sealed yet
    /// follow in their spine order.
    pub fn open(flit_dir: impl AsRef<Path>) -> crate::Result<Self> {
        let store = spine::Spines::new(flit_dir.as_ref());
        store.ensure_dirs()?;
        let loaded = store.load()?;
        let mut chains = Self::in_memory();
        let mut acts: HashMap<ActId, Act> = loaded.acts.into_iter().map(|a| (a.id, a)).collect();
        // sealed moments first, in the keeper's order
        for fact in loaded.facts {
            if let Some(act) = acts.remove(&fact.body.act_id) {
                chains.append_act_inner(act, false, None)?;
            }
            match chains.append_fact_inner(fact, false)? {
                AppendOutcome::Pending => return Err(crate::Error::Invalid("a stored fact seals an act that is not stored".into())),
                _ => {}
            }
        }
        // then everything unsealed, retrying until nothing more lands
        let mut rest: Vec<Act> = loaded.unsealed_order.iter().filter_map(|id| acts.remove(id)).collect();
        rest.extend(acts.into_values());
        loop {
            let before = rest.len();
            let mut again = Vec::new();
            for act in rest {
                match chains.append_act_inner(act.clone(), false, None) {
                    Ok(AppendOutcome::Pending) => {}
                    Ok(_) => {}
                    Err(_) => again.push(act),
                }
            }
            rest = again;
            if rest.is_empty() || rest.len() == before {
                break;
            }
        }
        chains.received_at = store.load_received();
        chains.store = Some(store);
        Ok(chains)
    }

    pub fn root(&self) -> Option<&Path> {
        self.store.as_ref().map(|s| s.root())
    }

    // ----- reading -----

    /// Number of acts (sealed or not).
    pub fn len(&self) -> usize {
        self.acts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.acts.is_empty()
    }

    pub fn genesis(&self) -> Option<&Act> {
        self.genesis.and_then(|g| self.acts.get(&g))
    }

    /// The song's id: its genesis act.
    pub fn song_id(&self) -> Option<Hash> {
        self.genesis
    }

    pub fn crew(&self) -> &CrewState {
        &self.crew
    }

    pub fn beings(&self) -> &Beings {
        &self.beings
    }

    /// The keeper (Name, machine). Before any Keeper act: the genesis Name
    /// on any machine.
    pub fn keeper(&self) -> Option<(Id, String)> {
        self.keeper.current.clone().or_else(|| self.genesis().map(|g| (g.name(), String::new())))
    }

    /// The exact app version the keeper seals: the song setting, or the
    /// version that made the genesis act.
    pub fn required_version(&self) -> Option<String> {
        self.histories.get(&main_history()).and_then(|h| h.view.settings.required_version.clone()).or_else(|| self.genesis().map(|g| g.body.app_version.clone()))
    }

    pub fn get(&self, id: &ActId) -> Option<&Act> {
        self.acts.get(id)
    }

    pub fn contains(&self, id: &ActId) -> bool {
        self.acts.contains_key(id)
    }

    pub fn fact(&self, id: &FactId) -> Option<&Fact> {
        self.facts.get(id)
    }

    /// The fact that sealed an act, if any.
    pub fn fact_for(&self, act: &ActId) -> Option<&Fact> {
        self.fact_of.get(act).and_then(|f| self.facts.get(f))
    }

    pub fn is_sealed(&self, act: &ActId) -> bool {
        self.fact_of.contains_key(act)
    }

    /// The history an act counts in: where its fact was sealed, else the
    /// history it was stamped on.
    pub fn history_of(&self, act: &ActId) -> Option<HistoryId> {
        if let Some(f) = self.fact_for(act) {
            return Some(f.history());
        }
        self.acts.get(act).map(|a| a.history())
    }

    pub fn has_history(&self, history: &HistoryId) -> bool {
        self.histories.contains_key(history)
    }

    /// All histories, main first, then by name.
    pub fn histories(&self) -> Vec<HistoryInfo> {
        let mut v: Vec<HistoryInfo> = self.histories.values().map(|h| h.info.clone()).collect();
        v.sort_by(|a, b| (!a.is_main()).cmp(&(!b.is_main())).then(a.name.cmp(&b.name)).then(a.id.cmp(&b.id)));
        v
    }

    pub fn history_info(&self, history: &HistoryId) -> Option<&HistoryInfo> {
        self.histories.get(history).map(|h| &h.info)
    }

    pub fn history_name(&self, history: &HistoryId) -> Option<&str> {
        self.histories.get(history).map(|h| h.info.name.as_str())
    }

    /// (parent, from) for a history that was started from another.
    pub fn history_base(&self, history: &HistoryId) -> Option<(HistoryId, ActId)> {
        let h = self.histories.get(history)?;
        Some((h.info.parent?, h.info.from?))
    }

    /// Histories started from `history`.
    pub fn children_of(&self, history: &HistoryId) -> Vec<HistoryId> {
        let mut v: Vec<HistoryId> = self.histories.values().filter(|h| h.info.parent == Some(*history)).map(|h| h.info.id).collect();
        v.sort();
        v
    }

    // ----- chains -----

    /// Head of a being's act chain on a history: (act id, act_seq).
    pub fn act_head(&self, being: &BeingRef, history: &HistoryId) -> Option<(ActId, u64)> {
        let v = self.act_chains.get(&(being.clone(), *history))?;
        v.last().map(|id| (*id, v.len() as u64))
    }

    /// A being's act chain on a history, in act order.
    pub fn act_chain(&self, being: &BeingRef, history: &HistoryId) -> Vec<&Act> {
        self.act_chains.get(&(being.clone(), *history)).map(|v| v.iter().filter_map(|id| self.acts.get(id)).collect()).unwrap_or_default()
    }

    /// Every (being, history) chain with its length, for sync.
    pub fn act_heads(&self) -> Vec<(BeingRef, HistoryId, u64)> {
        let mut v: Vec<_> = self.act_chains.iter().map(|((b, h), ids)| (b.clone(), *h, ids.len() as u64)).collect();
        v.sort();
        v
    }

    /// Acts of a being on a history after act number `after`.
    pub fn acts_after(&self, being: &BeingRef, history: &HistoryId, after: u64) -> Vec<&Act> {
        self.act_chains.get(&(being.clone(), *history)).map(|v| v.iter().skip(after as usize).filter_map(|id| self.acts.get(id)).collect()).unwrap_or_default()
    }

    /// A thing's fact chain on a history, in the keeper's order.
    pub fn fact_chain(&self, thing: &Thing, history: &HistoryId) -> Vec<&Fact> {
        self.fact_chains.get(&(thing.clone(), *history)).map(|v| v.iter().filter_map(|id| self.facts.get(id)).collect()).unwrap_or_default()
    }

    /// Number of sealed facts on a thing in a history (the next act's based_on).
    pub fn fact_seq(&self, thing: &Thing, history: &HistoryId) -> u64 {
        self.fact_chains.get(&(thing.clone(), *history)).map(|v| v.len() as u64).unwrap_or(0)
    }

    /// Sealed facts of a history in seal order.
    pub fn sealed(&self, history: &HistoryId) -> Vec<&Fact> {
        self.histories.get(history).map(|h| h.sealed.iter().filter_map(|id| self.facts.get(id)).collect()).unwrap_or_default()
    }

    /// Acts of a history nobody sealed yet, in arrival order.
    pub fn unsealed(&self, history: &HistoryId) -> Vec<&Act> {
        self.histories.get(history).map(|h| h.unsealed.iter().filter_map(|id| self.acts.get(id)).collect()).unwrap_or_default()
    }

    pub fn unsealed_count(&self, history: &HistoryId) -> usize {
        self.histories.get(history).map(|h| h.unsealed.len()).unwrap_or(0)
    }

    pub fn unsealed_total(&self) -> usize {
        self.histories.values().map(|h| h.unsealed.len()).sum()
    }

    /// Number of sealed facts in a history (the keeper's seal head).
    pub fn seal_seq(&self, history: &HistoryId) -> u64 {
        self.histories.get(history).map(|h| h.sealed.len() as u64).unwrap_or(0)
    }

    /// Every history's seal head, for sync ("moments after head X").
    pub fn seal_heads(&self) -> Vec<(HistoryId, u64)> {
        let mut v: Vec<_> = self.histories.iter().map(|(id, h)| (*id, h.sealed.len() as u64)).collect();
        v.sort();
        v
    }

    /// Sealed moments of a history after seal number `after`: (fact, act).
    pub fn moments_after(&self, history: &HistoryId, after: u64) -> Vec<(&Fact, &Act)> {
        let Some(h) = self.histories.get(history) else { return vec![] };
        h.sealed.iter().skip(after as usize).filter_map(|f| self.facts.get(f)).filter_map(|f| self.acts.get(&f.act_id()).map(|a| (f, a))).collect()
    }

    /// The acts of a history in fold order: sealed (seal order) then
    /// unsealed (arrival order).
    pub fn sequence(&self, history: &HistoryId) -> Vec<&Act> {
        fold::sequence(self, history)
    }

    /// Position of an act in its history's fold order.
    pub fn position(&self, act: &ActId) -> Option<(HistoryId, usize)> {
        fold::position(self, act)
    }

    /// The latest moment of a history (where "now" is), or for an empty
    /// history the moment it forks from.
    pub fn latest(&self, history: &HistoryId) -> Option<ActId> {
        let h = self.histories.get(history)?;
        if let Some(a) = h.unsealed.last() {
            return Some(*a);
        }
        if let Some(f) = h.sealed.last().and_then(|f| self.facts.get(f)) {
            return Some(f.act_id());
        }
        h.info.from
    }

    /// Current state of a history (sealed facts + unsealed acts).
    pub fn state(&self, history: &HistoryId) -> Option<&SongState> {
        self.histories.get(history).map(|h| &h.view)
    }

    /// State of a history from sealed facts only.
    pub fn sealed_state(&self, history: &HistoryId) -> Option<&SongState> {
        self.histories.get(history).map(|h| &h.sealed_state)
    }

    /// State of the history `at` is in, just after `at` was folded in.
    pub fn state_at(&self, at: &ActId) -> crate::Result<SongState> {
        fold::state_at(self, at)
    }

    /// Acts that are undone in a history right now.
    pub fn undone(&self, history: &HistoryId) -> HashSet<ActId> {
        fold::undone(self, history)
    }

    /// Is `act` in the past of `history` (in it, or in a parent before the fork)?
    pub fn in_history(&self, history: &HistoryId, act: &ActId) -> bool {
        fold::in_history(self, history, act)
    }

    /// The folded-in, not-undone acts of `history` up to and including
    /// `upto`, inherited parent acts first, in fold order.
    pub fn effective(&self, history: &HistoryId, upto: Option<&ActId>) -> Vec<&Act> {
        fold::effective(self, history, upto)
    }

    pub fn all_acts(&self) -> Vec<&Act> {
        let mut v: Vec<&Act> = self.acts.values().collect();
        v.sort_by_key(|a| a.id);
        v
    }

    pub fn all_facts(&self) -> Vec<&Fact> {
        let mut v: Vec<&Fact> = self.facts.values().collect();
        v.sort_by_key(|f| f.id);
        v
    }

    /// When this machine first received an act (display only).
    pub fn received_at(&self, act: &ActId) -> Option<u64> {
        self.received_at.get(act).copied()
    }

    pub fn evidence(&self) -> &[Equivocation] {
        &self.evidence
    }

    /// Beings the keeper holds (equivocation seen, no owner decision yet).
    pub fn held(&self) -> &BTreeSet<BeingRef> {
        &self.held
    }

    pub fn pending_count(&self) -> usize {
        self.pending_acts.values().map(|v| v.len()).sum::<usize>() + self.pending_facts.values().map(|v| v.len()).sum::<usize>()
    }

    /// Acts we are waiting for (ask peers for these).
    pub fn wanted(&self) -> Vec<ActId> {
        let mut v: Vec<ActId> = self.pending_acts.keys().chain(self.pending_facts.keys()).copied().collect();
        v.sort();
        v.dedup();
        v
    }

    // ----- writing -----

    /// Verify and add an act (ours or a peer's). Applied optimistically to
    /// its history's view; written to the Name's spine when there is a store.
    pub fn append_act(&mut self, act: Act, received_at: Option<u64>) -> crate::Result<AppendOutcome> {
        self.append_act_inner(act, true, received_at)
    }

    /// Verify and add a keeper's fact. Needs its act (else Pending).
    pub fn append_fact(&mut self, fact: Fact) -> crate::Result<AppendOutcome> {
        self.append_fact_inner(fact, true)
    }

    /// A MOMENT: the fact with its act.
    pub fn append_moment(&mut self, act: Act, fact: Fact, received_at: Option<u64>) -> crate::Result<AppendOutcome> {
        let a = self.append_act_inner(act, true, received_at)?;
        if a == AppendOutcome::Pending || a == AppendOutcome::Equivocation {
            return Ok(a);
        }
        self.append_fact_inner(fact, true)
    }

    fn append_act_inner(&mut self, act: Act, persist: bool, received_at: Option<u64>) -> crate::Result<AppendOutcome> {
        if self.acts.contains_key(&act.id) {
            return Ok(AppendOutcome::Duplicate);
        }
        match verify::act(self, &act)? {
            verify::ActCheck::Ok => {}
            verify::ActCheck::Duplicate => return Ok(AppendOutcome::Duplicate),
            verify::ActCheck::Pending(prev) => {
                self.pending_acts.entry(prev).or_default().push(act);
                return Ok(AppendOutcome::Pending);
            }
            verify::ActCheck::Equivocation(other) => {
                let being = act.being();
                self.evidence.push(Equivocation { being: being.clone(), history: act.history(), act_seq: act.body.act_seq, acts: [other, act.id] });
                self.held.insert(being);
                if persist {
                    if let Some(store) = &self.store {
                        store.append_act(&act)?;
                    }
                }
                self.evidence_acts.insert(act.id, act);
                return Ok(AppendOutcome::Equivocation);
            }
        }
        self.insert_act(act.clone(), received_at);
        if persist {
            if let Some(store) = &self.store {
                store.append_act(&act)?;
                if let Some(ms) = received_at {
                    store.record_received(&act.id, ms);
                }
            }
        }
        // acts and facts that were waiting for this one
        if let Some(children) = self.pending_acts.remove(&act.id) {
            for child in children {
                let _ = self.append_act_inner(child, persist, None);
            }
        }
        if let Some(facts) = self.pending_facts.remove(&act.id) {
            for f in facts {
                let _ = self.append_fact_inner(f, persist);
            }
        }
        Ok(AppendOutcome::Appended)
    }

    fn append_fact_inner(&mut self, fact: Fact, persist: bool) -> crate::Result<AppendOutcome> {
        if self.facts.contains_key(&fact.id) {
            return Ok(AppendOutcome::Duplicate);
        }
        match verify::fact(self, &fact)? {
            verify::FactCheck::Ok => {}
            verify::FactCheck::Duplicate => return Ok(AppendOutcome::Duplicate),
            verify::FactCheck::Pending(act) => {
                self.pending_facts.entry(act).or_default().push(fact);
                return Ok(AppendOutcome::Pending);
            }
        }
        self.insert_fact(fact.clone());
        if persist {
            if let Some(store) = &self.store {
                store.append_fact(&fact)?;
            }
        }
        Ok(AppendOutcome::Appended)
    }

    /// Index a verified act and apply it optimistically.
    fn insert_act(&mut self, act: Act, received_at: Option<u64>) {
        let id = act.id;
        let being = act.being();
        let history = act.history();
        if self.genesis.is_none() {
            self.genesis = Some(id);
        }
        // whole-song effects apply as soon as the act is here (they are
        // signed by the Name or an owner; the seal only records order)
        match act.payload() {
            Payload::Crew(a) => {
                self.crew.apply(a);
                if let CrewAction::Pardon { being } = a {
                    self.held.remove(being);
                }
            }
            Payload::Being(b) => self.beings.apply_being(act.name(), b),
            Payload::Grant(g) => self.beings.apply_grant(g),
            Payload::Flag(f) => self.beings.flags.push(FlagRecord { by: act.name(), act: id, being: f.being.clone(), kind: f.kind.clone(), note: f.note.clone() }),
            Payload::Keeper(k) => {
                let next = (k.name, k.machine.clone());
                if self.keeper.current.as_ref() != Some(&next) {
                    self.keeper.previous = self.keeper().filter(|c| *c != next);
                    self.keeper.current = Some(next);
                    for h in self.histories.values_mut() {
                        h.keeper_fresh = false;
                    }
                }
            }
            Payload::History(h) => {
                let base = fold::state_at(self, &h.from).unwrap_or_default();
                let mut base = base;
                base.applied = 0;
                let info = HistoryInfo { id, name: h.name.clone(), parent: Some(history), from: Some(h.from), created_by: Some(being.clone()) };
                self.histories.insert(id, HistoryChains::new(info, base));
            }
            _ => {}
        }
        if let Some(ms) = received_at {
            self.received_at.insert(id, ms);
        }
        let plain = !matches!(act.payload(), Payload::Undo { .. } | Payload::GoBack { .. });
        self.acts.insert(id, act);
        self.act_chains.entry((being, history)).or_default().push(id);
        let h = self.histories.get_mut(&history).expect("history exists (verified)");
        h.unsealed.push(id);
        if plain {
            let act = &self.acts[&id];
            h.view.apply(act);
            h.view.applied += 1;
        } else {
            self.refresh_view(&history);
        }
    }

    /// Index a verified fact: the act moves from unsealed to sealed (maybe
    /// into another history), the sealed fold advances.
    fn insert_fact(&mut self, fact: Fact) {
        let id = fact.id;
        let act_id = fact.act_id();
        let history = fact.history();
        let thing = fact.thing().clone();
        let act = self.acts[&act_id].clone();
        let act_history = act.history();
        let by_current = self.keeper().map(|(n, _)| n == fact.body.keeper).unwrap_or(false);
        self.facts.insert(id, fact);
        self.fact_of.insert(act_id, id);
        self.fact_chains.entry((thing, history)).or_default().push(id);

        // leave the unsealed list of the history it was stamped on
        let mut moved = act_history != history;
        if let Some(ah) = self.histories.get_mut(&act_history) {
            if let Some(pos) = ah.unsealed.iter().position(|a| *a == act_id) {
                ah.unsealed.remove(pos);
                if pos != 0 {
                    moved = true;
                }
            }
        }

        let plain = !matches!(act.payload(), Payload::Undo { .. } | Payload::GoBack { .. });
        let h = self.histories.get_mut(&history).expect("history exists (verified)");
        h.sealed.push(id);
        if by_current {
            h.keeper_fresh = true;
        }
        if plain {
            let mut st = std::mem::take(&mut h.sealed_state);
            st.apply(&act);
            st.applied += 1;
            let n = st.applied;
            if n % CHECKPOINT_EVERY == 0 {
                h.checkpoints.push((n, st.clone()));
            }
            h.sealed_state = st;
        } else {
            self.recompute_sealed(&history);
        }
        if act_history != history {
            self.refresh_view(&act_history);
            self.refresh_view(&history);
        } else if !plain || moved {
            self.refresh_view(&history);
        }
    }

    /// The view = sealed fold + unsealed acts.
    pub(crate) fn refresh_view(&mut self, history: &HistoryId) {
        let Some(h) = self.histories.get(history) else { return };
        let needs_full = h.unsealed.iter().filter_map(|a| self.acts.get(a)).any(|a| matches!(a.payload(), Payload::Undo { .. } | Payload::GoBack { .. }));
        let view = if needs_full {
            fold::replay(self, history, None, true)
        } else {
            let mut v = h.sealed_state.clone();
            for a in h.unsealed.iter().filter_map(|a| self.acts.get(a)) {
                v.apply(a);
                v.applied += 1;
            }
            v
        };
        if let Some(h) = self.histories.get_mut(history) {
            h.view = view;
        }
    }

    /// Rebuild the sealed fold and checkpoints of a history.
    pub(crate) fn recompute_sealed(&mut self, history: &HistoryId) {
        let (st, checkpoints) = fold::replay_sealed(self, history);
        if let Some(h) = self.histories.get_mut(history) {
            h.sealed_state = st;
            h.checkpoints = checkpoints;
        }
        self.refresh_view(history);
    }

    /// Keep a received-at note for an act (display only; the sidecar).
    pub fn note_received(&mut self, act: &ActId, ms: u64) {
        if self.acts.contains_key(act) && !self.received_at.contains_key(act) {
            self.received_at.insert(*act, ms);
            if let Some(store) = &self.store {
                store.record_received(act, ms);
            }
        }
    }
}
