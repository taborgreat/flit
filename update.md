# FLit UPDATE PACK - supersedes Addenda 1 through 13 and later notes

# (includes: Merge rename, stamped_at/received_at, equivocation,

# and the KEEPER model for facts)

Apply all of it. Some parts may already be in the code; re-check them.
Where this pack and existing code or older spec text disagree, THIS
PACK WINS. Update FLIT_SPEC.md and HANDOFF.md to match when done.

# ===================================================================== 0. FINAL WORDS (use exactly these; never the old ones)

Code names:

- Name the account, one root keypair. Holds authority.
- Being who acts under a Name. Kind: human | ai | scripted.
- Stamp the only verb. A being stamps an ACT; the keeper seals the
  matching FACT. One act + its fact = one moment.
- Keeper the one machine per song that seals facts (role, movable).
- Act what a being did; lives on that being's ACT CHAIN.
- Fact what happened to a thing; lives on that thing's FACT CHAIN.
- Gate a stamp that refuses or closes (decline, revoke, block).
- History what branches were. "main" is the main history.
- Track a mixer/instrument/bus/master track. No owner.
- Thing anything with a fact chain (song, song settings, track,
  device). Fact chains are written only by the keeper.
  User-facing words: Moment, History, Track, Being, Merge, Timeline,
  Go back, New history from here, Save version, Crew, Song code.
  NEVER show users: line, idea, branch, lane, stamp, commit, key,
  Merge.
  Retired concepts: lanes, lane owners, ownership handoff, "offer to
  owner", mix lane, lines, ideas, Assist/Partner as special types.

=====================================================================

1. # TIME IS INFORMATION, NEVER ORDER
   Order, conflicts, sync and checkpoints NEVER use time. Time is kept
   only so people can see when work happened and to check claimed being
   kinds.

- Order comes only from chains: act_seq (per being), and the KEEPER's
  fact_seq / seal_seq (section 3). Never by clock.
- Conflicts are decided by the keeper's seal order (first sealed wins).
  Checkpoints every N facts. Sync by heads ("after head X"). None of
  these read any time field.
- Two kinds of time are recorded, both display/evidence only:
  a. stamped_at: when the being stamped it, by the clock of the
  machine that stamped it. Inside the stamp, signed. The act and
  the fact are sealed in the same stamp, so this is one instant.
  Can be wrong or faked, but a lie is signed and attributable.
  b. received_at: when THIS machine first received the stamp, by this
  machine's own clock. Kept locally by every peer (NOT in the
  stamp, not synced as truth). Hard to fake, because other people's
  machines record it.
- Song position (bars/beats) is placement in the music, not time. It
  stays (Audio start, Notes, Presence.at).
- Local timers (settle rule, UI, retries, presence rate limit) stay
  local and unrecorded.
- Test/lint: no ordering, conflict, sync or fold code reads
  stamped_at or received_at.

# ===================================================================== 2. IDENTITY: NAME (root) AND BEINGS

- A Name is one keypair: the root. A new Name has no beings; first run
  asks to create one (choose kind + display name + color).
- Beings live under the Name: e.g. "Tabor" (human), "Kade" (human,
  same computer), "Tabor's Engineer" (ai), "Vocal Cleaner" (scripted).
- Acts attach to the Name, partitioned per being per history.
- Signing: by default the Name's key signs every stamp for any of its
  beings on the Name's own machine. Stamp carries name + being.
- Delegated being key (opt-in exception): for a being that runs on
  another machine (e.g. ai on a homelab). The Name certifies that
  being's key with a Being stamp on the Name chain, optionally scoped
  to listed songs. No expiry by time; lasts until revoked or scope
  ends. The machine holds only the being key, never the Name key.
- "Run as a being" mode: open FLit on another machine with a being
  pass (pairing: main machine "Add being on another device" shows QR;
  other machine "Run as a being" scans).
- Authority stays with the Name only: crew membership, grants to ai
  beings, creating/certifying/revoking beings, recovery. Beings never
  approve, grant, or certify.
- Revoked being: new stamps rejected; all its facts stay.
- Crew membership is per Name; approving a Name admits its beings.
  A crew owner may block one specific being without removing the Name.
- UI: top bar "Acting as: Tabor (human) v" switcher. Each being has
  its own color, tab/thread, lens, presence.

# ===================================================================== 3. MOMENTS: ACTS BY BEINGS, FACTS SEALED BY THE KEEPER

Two records, two signers (the dual seal):

Act stamp (made by a being; lives on its act chain):
Act {
id hash of everything below except sig
name Name public key
being being id
act_prev previous act id on this being's act chain (this history)
act_seq act number on this being's act chain (1, 2, 3...)
thing most specific thing touched (song | song settings |
track | device)
property the exact property changed (param id, clip edge, note,
mixer control...), or none for whole-thing acts
based_on fact_seq of that thing the being last saw
history history id ("main" or another)
app_version FLit version that made this act (+ format_version)
kind Setting | Mixer | Transport | Notes | Audio | Version |
Crew | Being | Grant | Flag | Keeper | History | Merge |
Chat | Presence | HostState | GoBack | Settings
(+ Recipe/Build later)
gate true if it refuses/closes
payload kind-specific, host-neutral where possible
pos song position (bars/beats) if relevant, else none
stamped_at stamper's clock, UTC (display/evidence only, never order)
sig signature by the Name key or the certified being key
}

Fact (sealed by the song's KEEPER; lives on the thing's fact chain):
Fact {
thing the thing
history history the act was sealed into (may differ from the
act's history if declined into another history)
fact_seq number on this thing's fact chain in that history
fact_prev previous fact on this thing's chain (one; always linear)
seal_seq keeper's song-wide seal number for that history
act_id the act being sealed
keeper keeper Name + machine id
keeper_sig countersignature
}

- Fact chains are written ONLY by the keeper: always one straight line.
  Everyone holds copies of all act chains and all fact chains.
- Things nest; each level has its own fact chain:
  song
  song settings (tempo, time sig, global/default settings,
  Reference DAW, Keeper)
  track (mixer, instrument, bus, master)
  device (each plugin/VST/instrument on that track)
  An act targets the MOST SPECIFIC thing touched (EQ knob -> that EQ
  device; fader -> the track; tempo -> song settings).
- Four tiers (match TreeOS): hash -> verify -> fold -> stamp/seal.
  - verify: act signature valid; act_prev/act_seq continue the being's
    chain; MAY check (section 5); facts carry a valid keeper_sig from
    the current keeper.
  - fold: replay fact chains (sealed facts) to state, from the nearest
    checkpoint; unsealed acts applied on top optimistically.
  - stamp/seal: being stamps acts; keeper seals facts.
- Facts are RECORDS, not derivations: the keeper's decision of which
  acts the song accepted and in what order. Never computed per peer.
- Keep this in one module (flit-core::seal) with a small interface so
  it can be swapped for the TreeOS factum crate later. Do NOT depend on
  TreeOS code now.

# ===================================================================== 4. KEEPER, LIVE FLOW, CONFLICTS (no track ownership)

Keeper role:

- One machine per song holds the KEEPER role. Default: the song
  owner's machine. An owner can move it to any machine (e.g. an always-
  on homelab peer) with a Keeper act on song settings. One at a time.
- Keeper unreachable: everyone keeps working; acts flow peer to peer
  and apply live; they wait unsealed. Panel: "N moments waiting to be
  sealed". Any owner may move the role to an online machine.
- All owners gone for good: no one may take the role, but every crew
  machine holds the full song, so any crew member can start a NEW song
  from their copy (full history, credit intact, new owner).
- Recommend 2+ owners per song, or keeper on an always-on peer.
  Live flow:

1. Being applies its own act locally at once.
2. Act gossips directly to everyone in that history; they apply it at
   once (optimistic). Live feel never waits for the keeper.
3. Keeper verifies and either:
   - SEALS it into the act's history: appends a Fact, broadcasts it.
     Peers mark the act confirmed (no visible change).
   - DECLINES it into another history (a gate fact) ONLY if based_on
     is stale AND the same property was sealed since based_on. Keeper
     seals it into history "<Being>'s version of <property>" branching
     at based_on. Peers move that change off their current history
     smoothly, with a notice: "Your change to Vocal EQ collided with
     Alice's. Saved as a history. [Listen] [Merge]". Nothing dropped.
4. Different properties never conflict: a stale based_on alone is fine.

- Any crew being may act on any track in any history, live (ai and
  scripted beings: section 5). No track owners. Coordination is
  social, through presence (section 9), not locks.
- Reverts are rare: only same-property collisions within one keeper
  round trip, or offline overlap.

# ===================================================================== 5. AI AND SCRIPTED BEINGS; MAY CHECK

- kind ai / scripted: stamps land in their own history by default.
  The Name may grant that being specific tracks in main (Grant stamp on
  the song chain, revocable). Granted = may stamp there directly.
- "Assist" = an ai being working in your session at your request.
  "Partner" = an ai being running its own loop/session, possibly on
  another machine with a delegated key. Same rules; just usage.
- MAY check on every incoming stamp: being certified and not revoked;
  its Name is current crew; being not blocked; if ai/scripted, target
  is its own/allowed history or a granted track; Crew/Grant/Being
  stamps only from the Name itself. Fail = reject, log, never apply.

# ===================================================================== 6. HISTORIES AND MERGE

- A new song has one history: main. Everyone in a history is live with
  everyone else in it. No push/pull.
- A being is in exactly one history at a time; live sync carries only
  that history.
- New history from ANY moment (even before someone else's change).
  Only beings who switch to it see it. Histories are live too.
- Merge (merge AND pull, one operation): take a whole history or
  chosen tracks FROM one history TO another. Any human crew being may
  do it; ai/scripted only into histories/tracks they are allowed.
  Lands as one stamp group, undoable as one. Same-property conflicts
  listed in the dialog with default winner; user can flip each.
- Pull from another song you're in: imports those things' FACT chains
  (fact chains import/export; act chains never). Original authors'
  Name + being stay on every imported fact.
- Go back: a GoBack stamp; undoable.

# ===================================================================== 7. LENSES AND ORDER (timeline views)

Each lens reads its own chain. Lenses change what you see and its
order; never the fold.

- Song (default): all facts in the history, keeper seal order
  (seal_seq).
- Track: that track + all its devices (subtree), fact order.
- Device: that device's fact chain.
- Song settings: that fact chain.
- Me: my current being's act chain, ACT order (act_seq).
- My Name: all my beings' acts merged causally.
- A being / a Name: their act chain(s), act order.
- Team: all crew beings' acts, ordered by the seal_seq of their facts
  (unsealed acts last, by arrival).
- Combine: e.g. Me x Track Drums = my acts landing in Drums subtree.
- Hover card number: fact_seq on thing lenses (#212 on Vocal EQ);
  act_seq on being lenses (#88 of Tabor).
- Lens picker: tree (Song > Song settings / Tracks > Devices) + Me /
  My Name / Team / each being. Names of tracks/devices come from the
  host via the bridge; ids are FLit's own, host-neutral.

# ===================================================================== 8. THE TIMELINE BAR (always docked at the bottom)

Always at the bottom on every screen; height resizable (default 96px).
[History: main v] [lens] [Acts|Facts|Time] [== tape ==] [Live] [Merge]

1. History switcher: dropdown of all histories, each row with avatars
   of beings in it (hover row = popover, section 9). Bottom item: "New
   history from this moment" (selected moment, or live edge).
2. Lens button: section 7.
   Axis toggle [ Acts | Facts | Time ] (segmented control):
   - Acts: marks spaced evenly in act order (being lenses default).
   - Facts: marks spaced evenly in fact order (thing lenses default).
   - Time: same marks placed on a real time axis by stamped_at
     (toggle to received_at). Gaps show when nobody worked; clusters
     show sessions.
     Display only: switching axes never changes order or fold.
3. Tape: moments as small marks in the acting being's color, in the
   lens's order. Versions = taller marks with a seal glyph. Chat = dots
   above the strip. Gates = hollow marks. New moments appear live at
   the right edge. Drag/scroll left = back (detaches from live; [Live]
   snaps back). Ctrl+wheel zoom.
   Hover a mark: being (color, kind badge), Name, what (kind +
   property, e.g. "Vocal / EQ band 2 gain -> -3.0 dB"), thing, history,
   moment number, and claimed time (+ when you received it).
   Unsealed acts show as outlined marks until their fact is sealed.
   Right-click: Go back to here | New history from here | Pin chat
   here | Show this being's moments.
4. Scrubbing ALWAYS folds the whole song up to the selected stamp:
   - Listen scrub (while dragging): instant audio of that moment from
     stored renders (nearest Version render + each track's nearest
     Audio chunks). No DAW involvement.
   - Exact view (on release/click): real fold shown in the DAW as a
     read-only preview via the bridge (level 0 hosts: Listen only).
     Live work untouched until Go back / New history from here.
5. Checkpoints: every Version is one; agent also keeps local folded
   snapshots every ~500 stamps per history (cache, deletable, rebuilt
   by fold). Not stamps.
6. Merge button: dialog with FROM = current history (preselected),
   TO = the history it came from (default main), [swap], both are
   dropdowns. Choose whole history or tracks (checklist with moment
   counts since the branch point). Conflicts per property with default
   winner, flippable. [Listen] preview. Confirm = one stamp group on
   TO, one grouped mark on the tape, undoable.
7. Status strip under it (22px): connection (direct/relayed/offline),
   beings online grouped by history, pending uploads.

# ===================================================================== 9. PRESENCE (host-neutral; bridges translate)

Presence IS recorded: it is an act, so rewinding shows where everyone
was and what they were looking at.

- Kind Presence. Lands on the being's own presence thing
  (/presence/<being>), written only by that being: never conflicts,
  never declined into a history. Keeper seals it like any act.
- Settle rule: stamped only after focus has rested ~500 ms (local
  timer), so fast clicking doesn't flood the chain. Live avatar
  movement between stamps is shown from the unsettled local signal and
  is not recorded.
- Hidden by default in every lens; lens toggle "Show presence" reveals
  the marks.
- Scrubbing / Show in DAW: the track map and status strip show where
  every being was at that moment (fold presence up to that moment).
- Shown in the DAW on rewind too (show_presence, if the bridge supports
  it): while scrubbing, tints follow the scrub head (rate-limited,
  colors only, no other DAW changes); on Show in DAW / Go back, tints
  show exactly where everyone was at that moment; on [Live], tints
  return to current positions. Same safety as live tints: originals
  restored, stripped before any save. Setting to turn off.
  Presence {
  name, being, color
  history
  view enum: mixer | arrange | notes | device | browser | other
  track track id or none
  device device slot or none
  region clip / pattern / note-range id or none
  at song position if known
  }
- Agent broadcasts on change (local rate limit), drops on disconnect.
- Panel: track map with colored avatars where each being is. Beings in
  a DIFFERENT history than you: hollow/dimmed avatar + history name.
- Status strip: beings grouped by history, e.g.
  [main: (T)(M)] [drop-v2: (S)] [vocal-fix: (A)(K)]
  Hover a group: popover listing each being (color, name, kind, Name,
  where they are) with [Switch to this history] [Follow <being>].
  Hover any single avatar anywhere: being + kind + Name + history +
  location.
- Follow: click an avatar -> your DAW navigates to their view/track/
  device (as far as the bridge allows). Toggle to keep following.
  HostBridge capabilities (optional, declared in capabilities()):
- report_presence(): host -> core (translate DAW focus).
- show_presence(list): core -> host, show others inside the DAW if
  possible. The tint itself is never stamped or synced as an edit,
  original host state restored, nothing left in saved projects.
- follow(presence): core -> host navigation.
  Missing = panel-only. Level 0 = FLit-only presence (which FLit
  instance/track the user touches).
  Add Presence to the Act kind list.
  FL bridge only: report via selected mixer track, selected channel,
  focused window, current pattern; show_presence = opt-in tint of track/
  channel color in the visitor's color, restored after, stripped before
  any save; follow = select same track/channel, open same window.

# ===================================================================== 10. MIXING, AUDIO FORMATS, PROCESS MODEL (from Addenda 1-2, updated)

Mixing across DAWs:

- Track kinds are descriptive: part | bus | master. No owners.
- Print point: a track leaving its host as audio is printed after its
  own inserts, fader and pan, before sends and master. Stored on Audio
  stamps/streams as print_point.
- Reference DAW: a song-settings value "Reference DAW: <being>", set by
  a Settings stamp. That DAW's master output is the Reference mix
  stream; panel toggle "Hear: Reference mix / My local mix"; final
  render of a history comes from the Reference DAW.
- If a being has processing on their own master and isn't the
  Reference DAW, panel says once: "Only you hear your master chain."
  Audio formats (never MP3):
- Listening copy: Opus ~160-256 kbps; live streams + sent first.
- Archive copy: FLAC; the true copy; sent in background or on demand.
- Must pull FLAC before export, processing, Merge, or editing.
- Chunks encoded per bar, content-addressed (BLAKE3).
- Audio stamp: track, pos, print_point, sample_rate, bit_depth,
  channels, opus_chunks[], flac_chunks[].
- Setting: "Download full-quality audio: always / on demand / Wi-Fi
  only". Recorder's original is never deleted.
  Process model:
- flit-agent: background helper process owning log, chains, blobs,
  iroh, encoding, identity. One per user per machine.
- Plugin (all modes) is a thin IPC client; auto-starts the agent.
  Agent crash: audio keeps playing, plugin shows "reconnecting".
- Audio callback never allocates, locks, or touches IPC; lock-free
  rings only.
- Per-peer quotas; fuzz targets for stamp decoding, Opus/FLAC chunks,
  HostState, IPC. Relay-only privacy option. Submix stream option.

# ===================================================================== 11. FL CHANGE DETECTION (FL bridge only)

1. Script listens: OnRefresh(flags), OnDirtyMixerTrack, OnDirtyChannel,
   OnIdle. FL says WHERE something changed, not WHAT.
2. Script diffs against its snapshot to get the exact change.
3. Agent applies the settle rule (local timer) and stamps once.
4. Notes/clips/trims not visible live: on save, diff the .flp with
   PyFLP; one stamp per difference.
5. Audio: plugin captures the take; one Audio stamp per take.

# ===================================================================== 12. VISUAL DESIGN

No toolkit defaults survive. All tokens in theme.rs; widgets read
tokens only.

- Type: embed IBM Plex Sans (UI) + IBM Plex Mono (values, moment
  numbers, ids). Sizes 11 / 12 / 14 / 18. Section labels 10-11px
  UPPERCASE +0.08em, muted. Numbers tabular, right-aligned. Regular +
  medium weights only.
- Shape: radius 3px controls, 4px panels. Never pills, never circular
  buttons (avatars/knobs only). Buttons 24px (28 primary). Segmented
  controls for tabs/toggles. 1px borders. No shadows, no gradients.
- Spacing: 4px grid; gap 8, section 16, padding 12. Dense, left-
  aligned. Regions: top bar 36px, left rail, main, Timeline bar,
  status strip 22px.
- Color:
  bg #0E0F11 surface #16181B raised #1E2125
  border #2A2E33 border-hi #3A3F46
  text #E6E8EB muted #8A9099 faint #5A606A
  accent #E8A33D (one accent only)
  ok #46A758 warn #E5A13D bad #E5484D (status only)
  Being colors: #5B8DEF #E07A5F #4FB286 #C77DDB #E8C547 #4CC3D9
  #F08FB0 #9AA5B1
- Icons: one set (Lucide or Phosphor), 16px, 1.5px stroke. No emoji.
- States: hover border-hi, pressed raised, focus 1px accent, disabled
  faint. Motion 80-120ms ease-out, hover/expand only.
- Plugin conventions: resizable, scale 75-200%, HiDPI, remembers size;
  double-click to type values, Shift-drag fine, Alt/Ctrl-click reset,
  tooltip exact value.
- Signature element: the Timeline tape (section 8). Everything else
  quiet.
- Deliverable: theme.rs + hidden "style gallery" of every widget in
  every state.

# ===================================================================== 13. STORAGE

See section 24 (the spine model). Section 24 is the storage law.

# ===================================================================== 14. MIGRATION FROM CURRENT CODE

1. Order by act_seq (acts) and keeper fact_seq/seal_seq (facts) only;
   add pos. Keep one
   stamped_at field (rename any old time field to it); add per-peer
   local received_at storage. Remove time from every ordering/sync
   path.
2. Event -> Act (name, being, thing, property, based_on, history,
   gate) + Fact records sealed by the keeper. Create one human being
   per existing identity; have the current machine act as keeper and
   seal existing events in their existing order.
3. Rename line/idea -> history; lane -> track. Delete lane ownership,
   Lane stamps, handoff UI, offer-to-owner flow, mix lane, single-
   writer assumption, Bring-in owner rule.
4. Split storage into act chains (per being per history) and fact
   chains (per thing per history), nested things.
5. verify module (MAY check + keeper_sig), fold module (replay sealed
   facts + optimistic unsealed acts, checkpoints), keeper module
   (seal / decline-into-history / role handoff), seal module.
   Remove multi-head fact merging and lowest-stamp-id tiebreaks.
6. Presence model + HostBridge report/show/follow; FL implementation.
7. UI: Acting-as switcher, Timeline bar, status strip, lenses, Merge
   dialog, theme.rs + style gallery.

# ===================================================================== 15. TESTS

- Same stamps in different arrival orders -> identical order per lens
  and identical fold on two peers.
- A peer with its system clock years off: identical order, fold and
  conflicts; only its Time-axis display differs.
- No time types in flit-core outside allowed local-timer modules.
- Dual seal: act signed by the being; fact countersigned by keeper.
- Fact chains have exactly one writer (keeper_sig on every fact).
- Keeper offline: acts apply live on all peers, seal on return.
- Keeper moved to another machine: sealing continues there.
- Two human beings under one Name keep separate act chains.
- Stamp signed by Name key for being X verifies; delegated key for
  being Y verifies only for Y; revoked key rejected.
- A being cannot issue Crew/Grant/Being stamps.
- ai being stamp on main without grant -> lands in its own history.
- Different properties on one track in parallel -> both sealed.
- Same property, stale based_on -> first sealed wins; the other is
  declined into "<Being>'s version of <property>" on every peer;
  nothing lost.
- Notes added to the same pattern in parallel -> both present.
- Merge: whole history and chosen tracks; one group; undoable; per-
  property conflicts; pull from another song imports facts not acts,
  credit kept.
- Track lens includes device facts; Me lens order = my act_seq; scrub
  identical from any lens; fold-to-stamp equals full replay.
- Agent killed mid-playback: audio continues, plugin reconnects.
- Validation rejects bad signature / unknown being / revoked / blocked.

# ===================================================================== 16. VERIFY ON WINDOWS (add to the list)

- FL undo history (general.getUndoHistoryCount etc.) as an act signal.
- Script can set + restore track/channel color; detect or precede a
  save to strip tints (else show_presence unsupported in FL).
- Script can select tracks/channels and open windows (Follow).
- Bridge can show a read-only preview state and return cleanly (or
  needs a separate preview project).
- FL can print a track at post-fader, pre-send without rerouting.

# ===================================================================== 17. ACTIVITY AND CLAIMED-KIND CHECKS

Purpose: see when a song was worked on, and let crew check that a
being claimed as human acts like one (and ai/scripted beings aren't
hiding as humans).

- Activity view (per being, from the Time axis): sessions, hours
  active, gaps, pace of moments, sessions with no breaks.
- Signals shown to crew (informational, never automatic punishment):
  - pace no human could keep (e.g. many precise edits per second for
    long stretches),
  - activity around the clock with no breaks,
  - stamped_at far from received_at on other peers (clock lies or
    delays),
  - identical timing patterns across beings.
- Flag: any crew Name can flag a being's claimed kind ("I think this
  is not human"). A Flag stamp on the song chain, signed, with an
  optional note; visible on that being's hover card and lens. The
  flagged Name can respond with a chat. A song owner decides what to
  do (nothing, ask, set the being to ai rules, block the being).
- Flags never change order or history; they are just moments.
- Add Flag to Stamp kinds. Tests: flag visible to all peers; signals
  computed from received_at on each peer independently.

# ===================================================================== 18. MERGE (renamed from Bring in) AND CONFLICTS BY EAR AND EYE

User-facing word is MERGE everywhere (button, dialog, notices, tape
marks). Code: Bring stamp kind -> Merge. Same rules as section 6.
Merge dialog, conflicts step (only shown if there are conflicts):

- One row per conflicting property, grouped by track > device.
- Each row shows BOTH sides visually, side by side, in each being's
  color: knob/fader value (e.g. -3.0 dB vs -6.5 dB), automation curve
  overlay, clip edges over the waveform, notes overlaid in the piano-
  roll strip, plugin settings diff.
- Each row has [A] [B] listen buttons: Listen scrub of the merged
  result with that side chosen, looping the affected region (song
  position of the change, or the whole track if global). Switching A/B
  is instant (both pre-rendered from stored audio + fold).
- Choose per row: Keep FROM | Keep TO. Bulk: "Keep all FROM" / "Keep all
  TO". Default winner pre-selected by section 4 rule.
- Optional: "Keep both" for clip/track conflicts = duplicate the track
  so both versions stay side by side to compare later.
- Final [Listen] previews the whole merged result before Confirm.
- Confirm = one Merge stamp group on TO, one grouped mark on the tape,
  undoable as one.
  Auto-conflict notices (section 4) also say Merge: "Saved as a history.
  [Listen] [Merge]".

# ===================================================================== 19. OFFLINE WORK AND EQUIVOCATION

Offline work:

- Acts made offline are stored and applied locally, then sent on
  reconnect. The keeper seals those that don't overlap newer sealed
  changes; overlapping ones are sealed into history "<Being> offline"
  (branching at the being's last based_on) and the being gets one
  notice with [Listen] [Merge]. No client-side re-sealing.
- Status strip shows "N moments not yet sent / not yet sealed".
- Once an act is published it is never changed.
  Equivocation (proof of cheating):
- Two validly signed acts by the same being with the same act_seq (or
  same act_prev) on the same history = equivocation.
- Any peer that sees both keeps both as evidence and broadcasts an
  automatic Flag (kind: equivocation) with both act ids. Peers verify
  the evidence themselves before showing it. The keeper stops sealing
  that being's new acts until a song owner decides (accept / block
  being / remove Name). Earlier facts stay.
  Tests: offline acts with no overlap sealed normally; with overlap,
  sealed into "<Being> offline"; two acts with same act_seq from one
  being -> flag on every peer, keeper holds further acts.

# ===================================================================== 20. FOLD VS THE DAW: APPLYING STATE WITHOUT WRECKING IT

- The core folds facts into an in-memory, host-neutral SONG MODEL
  (tracks, devices with FLit device ids, param values, notes, clips,
  settings). Cheap data. This is the fold.
- The DAW is a renderer of that model. Touching it is expensive, so do
  it rarely and minimally.
  Rules:

1. Scrubbing (dragging the tape) NEVER touches the DAW: Listen scrub
   only (stored audio).
2. Applying a moment to the DAW only on an explicit action: [Show in
   DAW] (read-only preview), Go back, New history from here, switching
   history, or Merge confirm. Never on hover or drag release.
3. Apply = DIFF, never rebuild. Compare the folded model at the target
   with the DAW's current state (as last reported by the bridge):
   - param differences -> set params only;
   - device present in both (same FLit device id) -> never reloaded,
     settings only;
   - devices that differ -> add/remove only those, batched, transport
     stopped, off the audio thread;
   - full device restore uses the plugin's saved-state chunk in one
     call, not param-by-param.
4. Big jumps -> SNAPSHOT PATH: if the diff exceeds a cost threshold
   (e.g. more than N device adds/removes, or a track structure
   change), load the nearest Version's project file in one load, then
   apply the remaining small diff. The agent estimates cost and picks
   the lighter path.
5. Apply is one atomic step from the user's view: progress shown,
   audio muted briefly if devices change, and it can be cancelled
   before it starts.
6. Level 0 hosts: no DAW apply; Listen scrub and audio only.
   HostBridge additions: read_state() (current DAW state in model form),
   apply_diff(diff), load_snapshot(project), estimate_cost(diff).
   VERIFY ON WINDOWS: FL script can add/remove plugins on mixer slots and
   restore plugin state chunks; cost of loading a project vs piecemeal
   changes; whether FL can stay running cleanly across many applies.
   Tests (FakeHost): repeated Go back / forward across 100 moments never
   reloads unchanged devices; big jump picks snapshot path; parameter-
   only jump touches params only.

# ===================================================================== 21. THE PROTOCOL: TWO VERBS, ACT AND MOMENT

Every message between machines (and between plugin, agent and bridge)
is one of two verbs. Same words locally and on the network.

- ACT send something new. The being's signed Act. Goes to peers
  (live, optimistic) and to the keeper. Every act creates a
  new moment once sealed. Join requests, chat, grants, flags,
  keeper moves, merges: all acts.
- MOMENT receive moments: the keeper's sealed Fact together with its
  Act, plus the blobs it points to. Used for live delivery of
  new moments, catching up after offline ("moments after head
  X"), joining a song, and filling any gap. Ask any peer;
  every copy is identical and verifiable.
  Presence is an ACT too (section 9): it creates moments on the being's
  own presence thing, hidden from normal lenses by default.
  Transport underneath is unchanged: iroh between machines, local IPC
  between plugin and agent, SysEx over the virtual MIDI port to the FL
  script (the port stays; it just carries Act / Moment messages).
  Tests: every state change on any peer traces to an ACT; a peer
  missing moments catches up using only MOMENT.

# ===================================================================== 22. UPDATES AND VERSION SWITCHING (production)

- Releases are published on GitHub Releases (public releases repo),
  built by CI for Windows / macOS / Linux, each signed with the FLit
  release key. The app verifies the signature before installing.
- Panel: Settings > Updates (and a small dot on the top bar when a
  newer version exists):
  - Current version, newest version, changelog for each release.
  - [Update] installs the newest.
  - Version list: every past release, marked Installed / Not
    downloaded. [Switch] to any version, downloading it if needed.
  - Channel: Stable / Beta.
- Versions install side by side; switching changes which one is
  active. Old installed versions kept until the user removes them.
- The plugin binary is locked while the DAW is open: updates to the
  plugin apply on next DAW restart (panel says so). flit-agent can
  update and restart itself immediately.
- Data compatibility: every song and chain store records a FORMAT
  VERSION. Newer app opening older data migrates forward (after an
  automatic backup). Older app opening newer data: opens read-only
  with a notice "This song was saved by a newer FLit"; never writes.
- SAME VERSION PER SONG (required):
  - Every Act records app_version (and format_version) inside the
    signed act. Hover card shows it; chains show which version made
    each moment.
  - Song setting "Required version" (exact app version), set by an
    owner with a Settings act. Default for a new song: the creator's
    version.
  - Keeper seals ONLY acts whose app_version equals the required
    version. Other acts are held unsealed (not declined, not lost) and
    the being sees "This song requires FLit X.Y [Update]".
  - Raising the required version: owner acts; every peer sees the
    notice; anyone on another version can still open, listen and
    scrub, but their new acts wait until they update (then resend).
  - The keeper's machine must run the required version, or the role
    must move.
  - Per song, not global: different songs may require different
    versions. With side-by-side installs, FLit activates the version
    each song requires when it is opened (if installed), or offers to
    download it.
- Update checks use the network only to fetch the release list; no
  account, no telemetry.
  Tests: switch to an older version and back without data loss; older
  version refuses to write newer-format songs; tampered release fails
  signature check; keeper holds acts from a non-required version and
  seals them after the being updates and resends; raising the required
  version shows the notice on every peer.

# ===================================================================== 23. NAME MANAGEMENT (top-left)

- Top-left of the panel: the current Name (and the acting being next
  to it, section 2). Click opens Name management:
  - All Names on this machine; [Switch] between them. Each Name has
    its own beings, songs, friends and crew memberships.
  - [New Name]: create a fresh Name (start over, or a second identity).
  - [Import Name]: restore from a recovery phrase or backup file.
  - [Back up Name]: show recovery phrase / export an encrypted backup
    file. Prompted once after creating a Name.
  - [Log out]: stop using this Name on this machine; keys stay in the
    OS keychain so logging back in is instant.
  - [Remove from this machine]: delete this Name's keys here (requires
    confirming the backup exists). Songs and chains on disk stay
    unless the user also chooses to delete them.
  - Beings of this Name, delegated being passes issued, and [Revoke].
- Losing a Name key with no backup = losing that Name's authority
  (owner rights, granting, revoking). The data still exists on every
  crew machine. Make backup hard to skip.
- Storage layout for multiple Names on one machine is decided later
  with the full storage design; keep Names fully separated.
  Tests: two Names on one machine never see each other's songs unless
  both are crew; log out / log in keeps everything; import from backup
  restores the same Name key and beings.

# ===================================================================== 24. STORAGE: THE SPINE (from TreeOS story.tree)

Truth is a small number of append-only files. Everything else is a
rebuildable sidecar.

Files per song (every crew machine holds a full copy):
.flit/
spines/<name>.tree ONE spine per Name: every act authored by any
of that Name's beings, in total order, across
all histories. Append-only, never rewritten.
spines/keeper.tree the keeper's spine: every Fact (seal) and
gate, in total order. Append-only.
blobs/<blake3> audio (Opus + FLAC chunks), project files,
plugin source. Content-addressed.
sidecar/ EVERYTHING derived (deletable, rebuilt):
reels/acts/<being>/<history> act chain views
reels/facts/<thing>/<history> fact chain views
heads/ folded present per reel
checkpoints/ folded snapshots
index/ lookup by act id, by thing, etc.

Spine format:

- One frame per line (JSONL now; a compact binary later is allowed if
  it round-trips byte-identically).
- Every frame carries its height (its line position on that spine;
  position is identity) and spine_prev (hash of the previous frame on
  the same spine). The frame body is the Act (or Fact) from section 3.
- The head is the last complete line. Append = write line + flush.
- Torn tail: if the last line is incomplete or fails its hash, it is
  skipped on open (crash mid-append heals by itself).
- Signatures: every act frame is signed (needed so single acts verify
  when they arrive over the network). The spine link (spine_prev) also
  gives each Name a running hash over its whole history; a later
  optimization may seal by turn (TreeOS) without changing the format.
- Witness data (stamped_at, received_at) is outside canonical content:
  never part of order, never required to rebuild a fold.

Sidecar law (from TreeOS): every derived structure is deletable and
must rebuild byte-identically by replaying the spines, or it does not
ship. Reels (act chains, fact chains) are projections built at open
from the spines, not separate truth.

Read path (fast): in-memory map -> heads/ file -> replay from nearest
checkpoint. Write path: append frame to the spine -> update the
affected reels/heads in sidecar.

Multiple Names on one machine (section 23): each Name's own keys and
settings live under its own profile; song folders hold every crew
Name's spine, so Names never mix data, only share songs they are both
in.

Tests: delete sidecar/ entirely -> reopen -> every fold, lens and
timeline identical; kill mid-append -> reopen -> torn line skipped,
fold clean; a copied spine from another peer verifies frame by frame
(signature, height, spine_prev) before use.

4.5 = xoming soon check again
alright here it is only worry once u are done withe verything else

# FLit PACK 2 - Organizations and Albums (+ frontend changes)

Apply after FLIT_UPDATE_PACK.md. Uses the same model: Names, beings,
acts, keeper-sealed facts, spines, sidecar law, no time in order.
Core primitive names stay as they are (Name, Being, ...). "Org" below
is the working term for the group; it is a UI word and may be renamed
later without touching the core.

=====================================================================

1. # ORGANIZATION (org): a standing group of Names
   Purpose: Names that work together keep working together across songs
   without re-adding each other on every song.

The org is a thing with its own chain:

- Its own spine and fact chain, sealed by its own KEEPER (one of its
  owners' machines; movable by an owner, same as a song keeper).
- Every member Name holds a full copy of the org's spine (sync with
  the same ACT / MOMENT protocol as songs).
- Org facts: name, color/icon, owners, members, pending invites,
  defaults for new songs (access, required version, keeper machine,
  ai policy).

Membership = countersigned (two acts):

- Owner acts: Invite <Name>.
- Invited Name acts: Accept (or Decline = a gate).
- Org keeper seals "<Name> is a member" only when both exist.
- Leave: one act by the member. Remove: one act by an owner.
- Promote/demote owner: one act by an owner. An org must keep at least
  one owner; the last owner cannot leave without promoting someone.
- Membership is per Name: it covers all of that Name's beings.
- The org NEVER controls a Name's beings. ai grants still come from
  each Name. The org may set a DEFAULT ai policy for its songs (e.g.
  "ai beings act in their own history only"), which a song owner can
  override per song.
- Flat for now: no orgs inside orgs.

Songs under an org:

- Song settings get one fact: org = <org id>, access = org | private.
  - access = org: every current org member is crew of this song,
    including songs that existed before they joined.
  - access = private: only the song's own crew, even inside the org.
- New song under an org takes the org's default access.
- Song owner may change access anytime (a Settings act; History shows
  when and who).
- Moving a song into or out of an org: a Settings act by a song owner.

MAY check (extends FLIT_UPDATE_PACK section 5):

- A Name may act in a song if it is in the song's own crew, OR the
  song's access = org and the Name is a current member of that org.
- Removed from the org: new acts in its open songs are rejected
  (unless the Name is also in that song's own crew). Everything already
  done stays.

===================================================================== 2. ALBUMS: tag songs into ordered collections
=====================================================================
An album is an ordered list of songs with a name. A song can be in any
number of albums. Albums never change songs; they only group them.

Two kinds:

- Personal album: belongs to one Name. Facts live on that Name's own
  spine (a "personal" thing). Synced only to that Name's own machines
  and delegated beings. Invisible to everyone else.
- Org album: belongs to an org. Facts live on the org's chain, sealed
  by the org keeper. Visible to all org members.

Album facts: create, rename, set cover image (blob), set description,
add song, remove song, reorder, archive.
Who may act:

- Personal album: only that Name (any of its beings).
- Org album: any member may add/remove/reorder songs they can access;
  only owners (or the album's creator) may rename, archive or delete.
  Visibility rule: a viewer only sees songs in an album that they have
  access to. A private song in an org album shows to others as a locked
  placeholder ("1 private song") or not at all (org setting).
  Albums are just facts: rewind, history and lenses work on them too.

===================================================================== 3. FRONTEND CHANGES
=====================================================================
Follow FLIT_UPDATE_PACK section 12 (theme.rs tokens, Plex fonts, no
pills, 4px grid). New and changed screens:

3.1 Left rail (Library)
SONGS all songs you can access (filter: mine / org / shared)
ALBUMS personal albums, then each org's albums
ORGS each org you belong to (+ "New org")
INVITES pending org and song invites (badge with count)

3.2 Top-left Name menu (FLIT_UPDATE_PACK section 23)
Add an "Orgs" list under the current Name: each org with role
(owner / member), [Open], [Leave].

3.3 Org page
Header: org name, color/icon, your role.
Tabs (segmented): Songs | Albums | Members | Settings

- Songs: org songs with access badge (org / private), [New song].
- Albums: org albums grid/list, [New album].
- Members: list of Names with their beings (colors, kind badges),
  role, [Invite] (paste/scan Name ID or QR), pending invites with
  [Cancel], owner actions [Promote] [Demote] [Remove].
- Settings: name, color/icon, keeper machine, defaults (access,
  required version, ai policy), [Leave org], [Archive org] (owner).

3.4 New song dialog
Under: [Me] or [<org>] dropdown.
Access (if under an org): [Org] [Private] segmented, default from
org settings.
Then the existing flow (New song / Share this project).

3.5 Song settings
New block "Org & access": org (none / pick), access toggle,
"Who can act here" summary (song crew + org members).
Album membership: chips for each album this song is in, [+ Add to
album] (personal or org albums you can edit).

3.6 Album page
Header: name, cover, description, owner (you or the org).
Ordered song list: drag to reorder, [Add song], remove per row.
Each row: song name, org badge, access badge, last activity (Time
axis display only), avatars of beings live in it now.
Locked rows for songs you can't access (if the org shows them).

3.7 Invites inbox
Each invite: from (Name + being), to (org or song), [Accept]
[Decline]. Accept/decline are acts.

3.8 Hover cards and badges
Song and being hover cards show org badge when relevant.
Timeline (FLIT_UPDATE_PACK section 8): unchanged; a song's org is
shown in the History switcher header as a small badge.

3.9 Empty states
No orgs: one line + [New org]. No albums: one line + [New album].
Keep them quiet; no illustrations.

===================================================================== 4. STORAGE (extends FLIT_UPDATE_PACK section 24)
=====================================================================
Per org (every member holds a copy):
orgs/<org>/spines/<name>.tree acts by member Names in this org
orgs/<org>/spines/keeper.tree org keeper's sealed facts
orgs/<org>/sidecar/ derived (members, albums, ...)
Personal albums: on the Name's own personal spine under the Name's
profile (synced only to that Name's own machines/beings).
Songs keep their own folders; the org link is a fact in the song.

===================================================================== 5. TESTS
=====================================================================

- Invite without accept: not a member; with accept: member on every
  member peer.
- Member gains access to an existing access=org song immediately;
  access=private song stays closed.
- Removed member: new acts in org songs rejected; old facts intact.
- Last owner cannot leave without promoting another.
- Personal album never appears on another Name's machine.
- Org album: member can add an accessible song; private song appears
  locked (or hidden) to non-crew members.
- Album reorder/rename/archive are facts: visible on the timeline of
  the album thing; rewind shows earlier album order.
- Song access change appears in the song's History with who changed it.

===================================================================== 6. THE SHELF: SELF-HOSTED SHARING AND FINDING PEOPLE (later)
=====================================================================
Not for the first build. Start simple; it reuses everything above.
Borrows TreeOS ideas: the Library / shelf, listings, covers.

6.1 Listing (publish): one act

- A Name or org lists a song or album on its shelf with a LISTING act:
  what (song / album / a specific Version), cover (blob), description,
  tags (genre, mood, roles wanted), and what others may do:
  listen - hear the published render only
  open - anyone may join and work (6.3)
- Unlisting is an act too. A listing points at a Version, so later
  private work is never exposed unless re-listed.
- What's published is audio (Opus listening copy; FLAC optional) plus
  the cover and listing facts. The song's chains stay private unless
  the listing is "open".

  6.2 Shelves are self-hosted (no central server)

- A shelf = the set of a Name's or org's listings, served by any
  always-on peer it chooses (homelab, a cheap VPS, a friend's box).
- Anyone can browse a shelf by Name ID, org ID or a shelf link.
- Following: you follow Names/orgs; your app pulls their listings
  (MOMENT sync of listing facts). Shown in plain order of listing, no
  ranking algorithm, no engagement feed.
- Community shelves (optional): anyone may run a shelf that collects
  listings submitted to it (e.g. a genre shelf, a local scene). Each
  is its own self-hosted peer with its own owners and rules.
- Studio page: a Name's public card: display name, beings shown as
  public (kinds visible: human / ai / scripted), orgs, listings,
  "open to collaborate" tags (roles, genres).

  6.3 Open songs (anyone can work on it)

- access = open (new value next to org | private).
- Anyone may join; newcomers are "guests": their acts land in their
  OWN history by default, never in main (like proposing changes).
- Only crew (song crew / org members) may Merge a guest's history
  into main. Credit travels with the facts, as always.
- Guests can be promoted to crew by an owner (one act).
- Protection: per-guest quotas, owner can block a Name or being,
  activity / claimed-kind flags (FLIT_UPDATE_PACK section 17) apply.

  6.4 Frontend (later)

- Left rail: SHELF (browse followed shelves, community shelves, search
  by tag), plus "My shelf" (your listings, [List song] [List album]).
- Song / album menu: [List on shelf] with listen | open, cover, tags.
- Studio page view for any Name; [Follow] [Invite to org] [Message].
- Open song badge on listings; [Join] creates your guest history.

  6.5 Order of building
  S1 list + listen (shelf served by your own always-on peer)
  S2 follow + browse + studio pages + tags
  S3 open songs with guest histories and crew merge
  S4 community shelves
  Tests (when built): unlisted Version stays private; guest acts never
  land in main; only crew can merge guest histories; following pulls
  listing facts only.

===================================================================== 7. STANDALONE APP + THIN PLUGIN (replaces "plugin is the panel")
=====================================================================
FLit is a standalone desktop app. The plugin is a thin connector.
(Extends FLIT_UPDATE_PACK section 10 process model: flit-agent becomes
the FLit app, with its own window.)

FLit app (standalone; the product):

- Owns all truth and work: spines, keeper role, sync, identity and
  keys, blobs, encoding, AI beings, MCP server (Door A) later.
- Full UI lives here: Library (songs, albums, orgs, invites, shelf),
  chat, Name management, updates, settings, full Timeline bar with
  lenses, Merge dialog, presence map.
- Runs without any DAW open: browse, listen (Listen scrub), chat,
  merge, manage orgs, run partners headless (homelab).
- Tray/menu-bar icon; starts on login (setting); one instance per user
  per machine.

FLit plugin (VST3 + CLAP, later AU; thin):

- Connects one DAW session to the app over local IPC. Modes: master
  (song/session link), lane-capture/stream per track, talk.
- Compact panel only: connection state, current song + history,
  avatars present, a slim Timeline strip (live marks, Live button),
  [Open FLit] which brings the app window to that song.
- Holds no truth. Plugin or DAW crash loses nothing. Auto-starts the
  app if it isn't running.
- Audio thread rules unchanged (lock-free rings only).
- Many DAWs at once: every open DAW's plugin connects to the same app.

Bridges: unchanged; they run in the app (and the FL script still talks
to the app over the virtual MIDI port).

Updates: the app updates freely; plugin updates are rare and apply on
next DAW restart (FLIT_UPDATE_PACK section 22).

Installer: installs the app, the plugin, and any bridge for DAWs it
finds. Users see one product.

Frontend: everything in FLIT_UPDATE_PACK section 12 and this pack's
section 3 is the APP's UI. The plugin panel uses the same theme.rs
tokens but only the compact elements above.

Tests: kill the DAW mid-session -> app keeps state, timeline intact;
kill the plugin -> app unaffected; two DAWs open -> both linked to one
app and one identity; app closed -> plugin starts it.

===================================================================== 8. MOVING FROM "PLUGIN IS THE APP" TO APP + PLUGIN (step by step)
=====================================================================
Goal: get from the current build (UI and logic inside the plugin) to
section 7 without ever breaking what works. Each step ends with the
whole thing still usable and all tests passing. Commit per step.

8.1 Shared pieces first (no behavior change)

- crates/flit-ui: theme.rs tokens, fonts, shared widgets (buttons,
  segmented controls, avatars, timeline tape, hover cards). Used by
  BOTH the app window and the plugin panel, so they always match.
- crates/flit-ipc: the plugin <-> app protocol (ACT / MOMENT messages
  - control messages: hello, link song, open window, status).
    Done when: plugin still works exactly as before, now built on flit-ui.

  8.2 App process, headless

- New binary: flit-app. Move everything that is not audio-thread or
  DAW-session work out of the plugin into flit-app: spines, keeper,
  sync (iroh), identity/keys, blobs, encoding, bridges.
- Plugin becomes an IPC client; auto-starts flit-app if missing.
  Done when: the plugin's current UI still shows everything, but all of
  its data comes from flit-app; killing the plugin loses nothing.

  8.3 App window shell (clean, empty frame first)

- flit-app opens its own window (same UI crate, e.g. eframe/egui,
  via flit-ui). Layout from FLIT_UPDATE_PACK section 12:
  top bar 36px: Name + Acting-as (left), song + history (center),
  update dot, settings (right)
  left rail: SONGS / ALBUMS / ORGS / INVITES (SHELF later)
  main area: the selected screen
  Timeline bar (bottom, always) + status strip 22px
- Tray / menu-bar icon; single instance.
  Done when: the empty shell opens, looks right in the style gallery,
  and shows connection + current song from flit-app.

  8.4 Move screens one at a time (plugin keeps working meanwhile)
  Order, each its own commit, each removed from the plugin only after
  it works in the app:
  1. Home: Your ID, New song / Share this project / Join with code
  2. Songs list + song page
  3. Friends + Invites
  4. Chat
  5. Timeline bar (full: lenses, axis toggle, scrub, right-click)
  6. Merge dialog
  7. Presence map + status strip groups
  8. Name management, beings, Acting-as, delegated passes
  9. Settings, updates, version switching

10. Orgs and Albums (sections 1-3 of this pack)
    Done when: every item above lives in the app; the plugin no longer
    contains any of them.

8.5 Plugin panel becomes compact

- Keep only: connection state, current song + history, avatars
  present, slim timeline strip (live marks + [Live]), mode selector
  (master / lane / talk), [Open FLit].
- [Open FLit] brings the app window forward on that song.
  Done when: the plugin panel fits in a small fixed window and uses only
  flit-ui components.

  8.6 Where everything we have so far goes
  Your ID, New/Share/Join ............... app (home)
  Songs, Friends, History, Chat tabs .... app
  Full Timeline, lenses, scrub, merge ... app (plugin: slim strip)
  Presence map, follow .................. app (plugin: avatars only)
  Acting-as switcher .................... app top bar (plugin shows
  the current being)
  Name management, beings, passes ....... app
  Settings, updates, versions ........... app
  First-run checklist ................... app (shows plugin + bridge
  status)
  Orgs, albums, shelf ................... app
  Master link to a DAW session .......... plugin (master mode)
  Track capture / streaming ............. plugin (lane mode)
  Talk sending / receiving .............. plugin (talk mode); voice
  via OS audio in the app
  FL bridge / FL script link ............ app
  Keeper role, sync, spines, keys ....... app

  8.7 Rules during the move

- One source of truth: the app. The plugin never writes spines.
- No feature exists in both places at once longer than one step.
- After every step: run all tests, open a real song, check the
  timeline and sync still work, then commit.
