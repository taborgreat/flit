# update.md progress notes (Claude session, 2026-10-10)

Working notes while applying `update.md` (the UPDATE PACK). Keep this file current: the session's context gets
compressed and this is how the work resumes. When everything below is done, go back to the MAIN THREAD.

## MAIN THREAD to return to (Tabor's FL test round, not yet reported)

Tabor was about to test, in FL Studio, with the build installed at 04:31 (agent/CLI/script current; the system
VST3/CLAP copy still needs one UAC approval or FL closed, see HANDOFF A11):

1. typing in panel fields (egui-baseview keyboard workaround + Paste buttons)
2. resizing the panel by its corner
3. Home link card: Project_1.flp "is this song"; open another .flp -> red "Edits are paused" card; "Make it a song"
4. Settings > Your name -> names in timeline/crew
5. add a plugin to a mixer slot, Undo the row -> slot switched off in FL + notice; undo the undo -> on
6. add an instrument, move a knob, Save version with a name -> channel track, knob rows, save row, note rows
7. scrub + Back to now; Go back; new history from a scrubbed moment; switch histories in the tree
8. Help tab feedback
   Tabor's known reports already handled: plugin add/remove not tracked (Device stamps), panel too narrow, auto-link,
   names as facts, help tab, "1.1" moments. HANDOFF.md A0..A11 is the log of the day.

## Decisions taken while applying update.md (pack wins over older text)

- Vocabulary in code: Name (root key = `Id`), Being (`BeingId`), Act, Fact, Keeper, History (`HistoryId`), Track
  (`TrackId`, no owners), Thing (song | settings | track | device | presence), Merge (was Bring), Moment = act+fact.
  UI words only: Moment, History, Track, Being, Merge, Timeline, Go back, New history from here, Save version,
  Crew, Song code. "main" is the main history.
- Order: act_seq per being per history; keeper fact_seq per thing per history + seal_seq per history. Time
  (`stamped_at` in the act, `received_at` local sidecar) is display only. Lint test: no time in order/fold/sync.
- Storage: `.flit/spines/<name-hex>.tree` (acts, JSONL frames {height, spine_prev, act}), `.flit/spines/keeper.tree`
  (facts), `.flit/blobs`, `.flit/sidecar/` (derived: received_at, checkpoints; deletable). Old `stamps/` + `chains/`
  songs: refused with a notice (dev-only data), no migration code.
- Keeper: default = song owner's machine; `Keeper` act on song settings moves it; agent seals when it holds the
  role; peers apply acts optimistically and mark them confirmed when the fact arrives. Decline = same property
  sealed since based_on -> fact sealed into history "<Being>'s version of <property>".
- Track ids stay the bridge's stable strings for now (fl-track-N, fl-channel-N); host-neutral ids are a later
  mapping layer (noted in HANDOFF).
- Device kind kept (plugin added/removed in a slot is an act on the track thing); Undo kept (Merge/GoBack are
  undoable); Profile replaced by the Being act's display_name/color.
- Fonts: IBM Plex Sans/Mono embedded if the files can be fetched; else system font fallback (theme.rs tokens).
- GitHub Releases / CI are Tabor's (never touch GitHub): the updater reads a manifest URL from settings; the data
  side (app_version/format_version in acts, Required version enforced by the keeper) is implemented.

## Phases and status

- [x] P0 this file
- [ ] P1 core rewrite (identity+beings, acts/facts/things, keeper, fold, spines, verify, histories, merge, presence,
      flags, equivocation, version fields) + section 15 tests
- [ ] P2 host/agent/net: ACT/MOMENT protocol, keeper in agent, optimistic apply, received_at, presence report,
      read_state/apply_diff/estimate_cost/load_snapshot (FakeHost), FL bridge: presence report, no owners
- [ ] P3 plugin UI: theme.rs + gallery, timeline bar + status strip, acting-as switcher, lenses, Merge dialog,
      Name management, Settings > Updates
- [ ] P4 docs: FLIT_SPEC.md (consolidated), HANDOFF.md, plan.md pointer, VERIFY list
      p[4.5] look at update.md for new thing
- [ ] P5 build/test/install, then MAIN THREAD

## Where I am right now

P1 starting: writing the new core modules under crates/flit-core/src (act.rs, thing.rs, seal/{mod,hash,verify,
fold,keeper,store}.rs, state.rs, song.rs).
