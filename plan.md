# FLit - System Spec (working name)

Author: Tabor Holly - 2026-10-09

## READ FIRST: how this gets built

Two Claude Code sessions, two machines.

- SESSION 1 (this one, Linux, NO FL Studio): build everything that does not
  need a real DAW. Test it all against a simulated DAW ("FakeHost").
- Then this folder is copied as-is to a Windows machine with FL Studio.
- SESSION 2 (Windows, FL installed): read HANDOFF.md, build the FL bridge
  for real, run the FL-only checks, get it working inside FL.

THE TWO RULES FOR SESSION 1:

1. The core knows nothing about any DAW. No FL types, names or concepts
   in flit-core, flit-net or the plugin's core logic. Every DAW is reached
   through one Rust trait, HostBridge. FL is the first bridge, not the
   design. The core crates must never depend on a bridge crate.
2. Never guess DAW behavior outside its bridge. On Linux everything runs
   against FakeHost. The FL bridge is written from docs, with every
   assumption marked: // VERIFY ON WINDOWS

Session 1 scope: phases L1-L7 below (milestones M0-M2 incl. text chat,
plugin shell, and lane streaming built so voice/Talk can reuse it).
Do NOT start recipes, AI control, partners, voice/Talk, or bridges for
other DAWs. Just keep the core general enough that they plug in.

## What FLit is

A way for people and machines to work on one song together, each in
their own DAW. FL Studio is the first DAW it supports.

1. Teamwork: several producers in one song, live, each in their own DAW.
   One on FL, one on Ableton, one on Logic is a valid crew.
2. Memory: every knob turn, trim and take is kept. Go back to any moment,
   or try an idea from there without touching the main song.
3. AI control that can't go unhinged: an AI edits through the same log as
   a person, so every move is visible and undoable.
4. AI partner: a second DAW session run by an AI joins as a crew member.
   You record vocals; it trims/compresses/EQs them on its own lane.
5. Talking: text chat, voice chat, and sending any mixer track live to
   each other. A teacher hears the student's guitar, the student hears the
   teacher's, and both can record it and keep working in their DAW.

Underneath: a song is an append-only log of signed events + a store of
files addressed by hash. People and machines are both just IDs writing to
that log, and every DAW is just a bridge into it. No server, no account,
no port forwarding.

## Design rules

- One installer, one thing to open (the FLit plugin in your DAW).
- The DAW stays the instrument. FLit watches it and responds.
- Plain words for users: ID, Song, Crew, Save version, Go back, Try an
  idea, Bring in. Never repo/commit/branch/key in the UI.
- Send the cheapest thing that sounds right: a setting if the peer can
  reproduce it, a live audio stream if not, full-quality files on save.
- Audio is the universal language. Anything one DAW can't reproduce from
  another arrives as audio. Mixed-DAW crews always work.
- Nothing is overwritten. History only grows.
- One owner per lane.
- Everything is an event. If a feature can't be events on the log, it
  doesn't go in.
- Core is DAW-neutral. Anything DAW-specific lives in that DAW's bridge.

## Words

- ID: user's public key, shown as short code + QR. Safe to share.
- Song: the shared work = every lane + full history. Every crew member
  holds a complete copy. No master copy.
- Song code: song's public ID. Lets someone ASK to join, never lets them in.
- Crew: IDs allowed in a song, signed into its history. Humans or AIs.
- Owner: song creator. Approves/removes crew, can promote other owners.
- Host: the DAW a crew member is using (FL, Ableton, Logic, Reaper...).
- Bridge: the per-DAW adapter between a host and the FLit core.
- Save version: named checkpoint (commit).
- Main: the default branch. Branch: a separate line of work forked from any
  moment (was "line"/"idea" in earlier drafts; code and UI say branch now,
  2026-10-10).
- Bring in: take a lane from an idea OR from another song you're in into
  this one. One operation for both merge and pull.
- Lane: one part (drums, vocals). One owner per lane per line. A lane is
  edited in its owner's host and heard by everyone in theirs.
- Recipe: plugin source + lockfile stored in the song so all can build it.
- Assist: AI acting AS a user, under the user's ID.
- Partner: AI acting as ITSELF, own ID, own DAW session, approved as crew.
- Chat: text messages in the song. Logged, so they live in History.
- Talk: live audio between people that is NOT part of the song (voice, or
  any mixer track sent to someone). Not logged unless someone records it.

## Hosts and bridges (keep the core general)

The FLit plugin itself (VST3 + CLAP, later AU via clap-wrapper for Logic)
loads in almost any DAW. A bridge adds deeper control on top. Each host
reaches whatever level its bridge supports; the core handles the rest.

Bridge levels:

- Level 0, plugin only: NO bridge. The plugin on each lane streams that
  lane out and plays others in. Tempo and position come from the plugin
  host's transport info. Saves capture rendered audio. Works in any DAW
  that loads the plugin, on day one, with zero DAW-specific code.
- Level 1, live control: bridge reports and applies parameter, mixer,
  transport and tempo changes live.
- Level 2, project: bridge reads the host's project file for versions,
  notes, clips and arrangement.
- Level 3, AI: bridge reports focus and exposes host-specific tools.

Planned bridges:

- FL Studio: levels 1-3. Python MIDI script + SysEx + PyFLP. FIRST.
- Ableton Live: levels 1-3 later. Remote Scripts or Max for Live for live
  control; .als project files are gzipped XML, easy to read.
- Logic, Reaper, Bitwig, others: level 0 immediately, deeper as bridges
  get written (Reaper has ReaScript, Bitwig has a controller API).

What crosses between different hosts:

- Audio: always (live Opus + saved chunks).
- Tempo, transport, song position: always (host-neutral).
- Notes: as MIDI, host-neutral, when both bridges are level 2.
- Plugin settings: only between peers with the same plugin, same version,
  any host (a VST3 param is the same in FL and Ableton).
- Host-only state (FL routing, Ableton racks, project file itself):
  stays with the lane's owner. Other hosts hear the result as audio.

Mixing across DAWs: DAWs mix differently (pan law, default FX, master
chains), so the song's final sound comes from ONE DAW per line.

- Lane kinds: part | bus | mix. Exactly one mix lane per line = the
  master chain. Its owner is the "mix owner"; their DAW is the reference.
  Handed over with the normal Lane ownership event.
- Print point: a lane leaving its host as audio is printed after its own
  inserts, fader and pan, before sends and master. Sends to shared buses
  are not baked in; buses are their own lanes.
- Reference mix: the mix owner's master output is a stream anyone can
  hear. Panel toggle "Hear: Reference mix / My local mix".
- Final render/export of a line = the mix owner's render.
- If a non-mix-owner has processing on their own master, the panel says
  once: "Only you hear your master chain. The song's master belongs to
  <mix owner>."

Mixed crew example: Tabor on FL, Arlo on Ableton, Max on FL. Tabor and Max
see each other's FL settings live. Arlo hears both their lanes as audio in
Ableton; they hear Arlo's lanes as audio in FL. Everyone's History shows
everything. Go back and ideas work for all three.

## User flow

Installer silently: installs the plugin into the standard plugin folders,
plus the bridge for any supported DAW it finds (for FL: copy the script
into FL's hardware scripts folder, loopback MIDI driver on Windows).
First open: add FLit on the master track -> ID created, private key in OS
keychain -> if a bridge exists for this host, a one-time checklist to
enable it (FL: pick "FLit" in MIDI settings). No bridge = level 0, still
works.
Home screen: Your ID (Copy, QR) + three buttons:

- New song
- Share this project (current project becomes a song, v1)
- Join with a song code (sends request to owner)
  Tabs: Songs, Friends, History, Chat. A Talk button sits in the top bar.

## Talking: chat, voice, lessons

Everything here reuses pieces FLit already has: Chat is one more event
kind, Talk is the same Opus-over-iroh streaming used for lanes.

Chat (text):

- Chat tab in the panel, one conversation per song, everyone in the crew.
- Messages are Chat events in the log, signed like everything else. They
  show in History beside the edits, so "why did we change the bass?" is
  answered right next to the change.
- Each message records the sender's line and song position. Click a
  message to jump to that moment.
- Pin a message to a History row ("try this from here").
- AI partners read chat like any event: "@partner redo take 3 tighter"
  becomes a task. AI assists never post to chat unless asked.
- Chat on an idea is visible to the whole crew, tagged with the idea's
  name, so private work stays private but talking about it doesn't hide.

Voice (simple):

- Talk button in the top bar: push-to-talk or open mic, per-person volume
  and mute, deafen.
- Default path uses the computer's own mic and headphones directly (OS
  audio), never touching the DAW mix. Nothing leaks into the song or a
  render. Headphones recommended; the checklist says so once.
- Voice is live-only. It is never logged.

Mixer talk (lessons, jamming, monitoring):

- Put FLit in Talk mode on any mixer track. Whatever reaches that track
  (guitar through amp sim, a mic with effects, a synth, a bus) is sent
  live to the people you pick.
- On their side it arrives on a FLit Talk receiver they put on any track
  in their own mixer, so they can add effects, level it, solo it.
- Talk tracks are marked as outside the song: FLit never renders them
  into versions, and they don't need a lane owner.
- Record it: either side can hit record on a received Talk stream. The
  recording becomes a normal Audio event on a lane the recorder owns,
  credited to the person who played it. A lesson riff becomes a take.
- Lesson mode: lowest-latency settings (Opus low-delay, small buffers,
  direct connection preferred over relay). Internet delay makes it best
  for call and response: teacher plays, student plays back, both hear
  each other clearly with their own instrument tone.
- Works at bridge level 0: it is just the plugin on a track, so a teacher
  in Ableton and a student in FL can do it.

Lesson example: teacher puts FLit Talk on the guitar track, picks the
student. Student hears the teacher's guitar tone on a track in FL, plays
back through their own Talk track. Both talk over voice. Student records
the teacher's riff into their project, slows the tempo, loops it, and the
teacher watches the student's edits live in History.

## Connecting (SSH-like, no server)

- Join door 1: owner shares song code -> friend pastes it -> owner sees
  friend's ID and taps Approve -> approval signed into song -> files sync.
- Join door 2: owner adds friend's ID -> friend taps Accept.
- Either way one side asks, other approves. Nothing joins automatically.
- Add friend: paste/scan ID, give a name.
- Block: your app refuses that ID everywhere. Private.
- Remove from crew (owner): signed removal, all copies stop syncing it.
- Promote to owner, Leave song.
- Only the open song syncs live; others catch up in background.

## Architecture

On each machine, inside the DAW:

- FLit plugin on master: panel UI, event log, versions, blob store, peer
  network (iroh), identity from OS keychain. Host-neutral.
- FLit plugin on lanes (same plugin, lane mode): streams that lane's audio
  out as Opus; receiver copy plays a friend's lane in. Host-neutral.
- FLit plugin in Talk mode (same plugin again): sends or receives a live
  track that is outside the song. Voice uses OS audio directly (cpal).
- Bridge (per host, optional): for FL, a Python MIDI script reporting and
  applying changes. FL Python can't open sockets, so script <-> plugin
  talk via SysEx over a loopback MIDI port. Other hosts use whatever
  their scripting allows; the core doesn't care how.

Networking: iroh (Rust).

- Node ID = public key. Identity and address are the same thing.
- NAT hole punching over QUIC, no ports. Relay fallback forwards encrypted
  bytes only (iroh public relays default, self-host later).
- 3+ people: iroh-gossip, nobody hosts.
- Files: iroh-blobs, BLAKE3 content addressed, resumable.
- Offline: on reconnect ask for events after last known head. Optional
  always-on headless peer (homelab) keeps songs reachable.

## Data model (host-neutral)

Log lives in a .flit folder beside the project.

Event {
id hash of fields below
parent previous event on this line
author writer's ID (public key)
sig signature
time wall clock + song position (bars/beats, host-neutral)
lane lane id
host which host produced it (fl, ableton, fake...) for display/debug
kind see list
payload kind-specific, host-neutral where possible
}

Plugin identity is host-neutral: format (vst3/clap/au) + vendor + name +
version. Parameters are addressed by the plugin's own param id and stored
as normalized 0..1 values. This is what lets an FL user and an Ableton
user with the same VST share settings.

Kinds:

- Setting: lane, device slot, plugin identity, param id, normalized value
- Mixer: lane volume/pan/mute (host-neutral); host-only routing goes in
  an opaque HostState payload only that host's bridge reads
- Transport: play/stop/position/tempo/loop
- Notes: lane, clip id, MIDI notes (host-neutral)
- Audio: lane, start position, print_point, sample_rate, bit_depth,
  channels, opus_chunks[] and flac_chunks[] (same bar boundaries)
- Version: host + hash of that host's project file + manifest of files it
  uses, name. Each member's version covers their host project; the song's
  version is the set of all members' latest versions at that point.
- Crew: approve/remove/promote an ID, signed by an owner (every copy
  verifies signer is a current owner)
- Lane: lane, new owner, kind (part | bus | mix). Each line has exactly
  one mix lane, created with the song, owned by the creator.
- Idea: name, event it starts from
- Bring: lane, source line (idea or other song), source event
- Chat: text, sender's line + song position, optional pinned event id.
  (Voice and Talk streams are never events; a recording of one becomes
  a normal Audio event credited to the player.)
- HostState: opaque host-specific blob + host tag. Core stores and syncs
  it but never interprets it. Escape hatch that keeps the core clean.
- Recipe / Build / Assist: later milestones

Blobs: audio, project files, plugin source stored by BLAKE3, audio cut
into bar-sized chunks so small edits make few new chunks.

Audio formats (never MP3; re-encoding degrades takes that get reused):

- Listening copy: Opus ~160-256 kbps stereo. Live streams use it, and
  every saved take is sent as Opus first so peers hear it fast.
- Archive copy: FLAC, lossless, ~50-60% of WAV. The true copy in
  history. Sent in the background after Opus, or fetched on demand.
- A peer must pull FLAC before export, processing a lane (AI partner,
  effects), Bring in, or editing that audio. Playback never needs it.
- Each chunk encoded independently per bar: addressable, resumable.
- Setting: "Download full-quality audio: always / on demand / Wi-Fi
  only". Default always, in background.
- The recorder's machine keeps the original recording at full
  resolution; FLit never deletes it.

## TreeOS act model (Addendum 3, applied on Windows)

The core uses the TreeOS model. Code vocabulary: Name (keypair), Stamp
(the only verb), Act (on the Name's ACT CHAIN, one per Name per line),
Fact (on the touched thing's FACT CHAIN, one per lane and one for the
song, per line), Gate (a stamp that refuses/closes). Dual seal: one
stamp, one signature, appended to both the author's act chain and the
thing's fact chain. Order = chain position; wall clock is metadata.
Stamp = Event plus: thing, fact_prev, gate. Lane fact chains are single-
writer per line (one owner per lane), so always linear. Song chain may
have multiple heads; next stamp cites all heads seen.
Four tiers: hash, verify (signature, act_prev, fact_prev, MAY check),
fold (replay facts to state; Go back = fold earlier + GoBack stamp),
stamp. Bring in imports FACT chains only; act chains never export.
Storage: .flit/chains/acts/<name>/<line>.log,
.flit/chains/facts/<thing>/<line>.log, .flit/stamps/<id>.
Lives behind one module (flit-core::seal) so it can be swapped for the
TreeOS factum crate later. Do not depend on TreeOS code now.

## Live sync and saves

Live, every edit:

1. Bridge (or plugin, at level 0) sees a change.
2. Settle rule: wait until control is still ~150 ms, keep only final value.
   1000 twists = 1 event. Knob returned to start = 0 events.
3. Send to everyone on the same line.
4. Receiver asks, in order:
   a. Have this plugin, same version, and a bridge that can set params?
   -> apply setting.
   b. Recipe for it in the song? -> build it, then apply (later milestone).
   c. Else -> play owner's live Opus stream for that lane.
   HostState: applied only if receiver is the same host; else ignored and
   the lane is heard as audio.
   On Save (host save, or the button):
5. Project file changed on disk -> Version event.
6. Render only lanes changed since last version, at the print point,
   chunk per bar, encode Opus + FLAC, hash.
7. Send Opus chunks peers don't have first, FLAC after in background.
   Costs: settings = bytes; Opus ~128 kbps/lane (10 lanes ~1.3 Mbps);
   saves = small Opus first, FLAC (about half of WAV) after.

## History, lines, Bring in

- Go back: click any History row, preview, "Go back to here". Load nearest
  Version before it, replay settings after it. Going back is an event,
  so it is undoable. Each member's host restores its own project; lanes
  from other hosts restore as their audio at that point.
- Captured live (level 1+): knobs, faders, mutes, tempo, transport.
  Captured per save (level 2+): notes, clips, trims, arrangement.
  Recorded/rendered audio: per save, every level.
- A new song has one line: Main. Everyone on Main is live. No push/pull.
- You are on exactly one line at a time. Live sync only carries your line.
- Try an idea from ANY History row, even before someone else's change.
  Only people who switch to that idea see it. Ideas are live too.
- Lanes are owned per line.
- Bring in works lane by lane, from an idea or another song you're in.
  Same lane changed on both sides: no knob-level 3-way merge; pick one,
  or keep both as two lanes.
- WHO can Bring in: the owner of the destination lane on the destination
  line, or a song owner. Others offer it ("Sarah has an idea for Drums"
  - listen button). Same rule for humans and AI partners.
- Bring in lands as one group in History; one click undoes the group.
- Pulling from another song keeps original author in history.
  Example: Bob changes drums on Main. Sarah clicks the row before Bob's
  change, tries an idea, takes Drums there, works alone, offers it. Bob
  (owns Drums on Main) listens, clicks Bring in. Everyone on Main hears it.

## Later milestones (NOT session 1)

Recipes (M7): custom plugins stored as source + lockfile in the song.
FLit only ever moves source it can build, never binaries, so commercial
plugins can't leak. No filter. Ships an nih-plug template. Builds are
reproducible; prebuilt binary accepted only if hash matches the recipe.
Build locally, first build asks once. Recipes are host-neutral: a recipe
built on Mac loads in Ableton, the same recipe on Windows loads in FL.

AI control (M8-M10): two doors, one tool layer.

- Door A: FLit runs a local MCP server; user's own Claude Code/Claude app
  connects (user's subscription, no keys in FLit). Build first.
  (Anthropic doesn't allow third-party claude.ai login; Door A avoids it.)
- Door B: built-in chat + MCP client to Ollama or any OpenAI-style API.
- Tools come in two layers: core tools (history, versions, ideas, lanes,
  settings by plugin param) work in every host; host tools come from the
  bridge (FL mixer, FL piano roll...). A level 0 host gets core tools only.
- Focus: AI only gets 10-15 tools for the section in focus (mixer,
  piano roll, playlist, plugin editor, recipes). Bridge reports focus;
  MCP server swaps tool list (list_changed). Users can pin focus.
- Assist edits land in an idea by default, only user's lanes, badged,
  one click rolls back a whole AI turn.
  Partners (M11): Door B engine headless, own ID, own DAW session (same box
  or another machine). Approved as crew. Owns lanes. Loop: watch log ->
  pick one task -> set focus from task -> work -> save version with note ->
  watch. Multi-section work = several tasks in a row. Job = short text on
  its crew entry. First job: process every vocal take into Vocal Processed.

## Folder layout

flit/
HANDOFF.md written by session 1, read first by session 2
Cargo.toml Rust workspace
crates/
flit-core/ events, log, blobs, versions, lines, bring in.
NO host code. Never depends on any bridge crate.
flit-net/ iroh: IDs, song codes, crew, live sync, blobs
flit-host/ HostBridge trait + capability levels
flit-plugin/ nih-plug VST3+CLAP: panel, master mode, lane mode,
talk mode (later). Works at level 0, no bridge.
bridges/
fake/ FakeHost: simulated DAW for all tests
fl/ FL bridge: Rust side + fl-script/device_FLit.py + flp/ PyFLP helpers
(ableton/, reaper/... later, same shape)
tools/
fake-session/ FakeHost demo/test driver

## HostBridge trait (in flit-host)

Every bridge implements the same trait and declares its level.

- capabilities(): which levels and features it supports
- on_param_changed (host->core): plugin identity, param id, value
- set_param (core->host)
- mixer / transport / tempo (both directions)
- on_focus_changed (host->core), level 3
- on_project_saved (host->core): path to project file, level 2
- read_project (level 2): notes, clips, arrangement in host-neutral form
- render_lane (core->host): or fall back to plugin capturing its own lane
- apply_host_state / capture_host_state: opaque HostState blobs
  FakeHost implements all of it in memory (emits knob sweeps, writes a
  sample project file, returns test WAVs). Tests also run FakeHost at
  level 0 to prove plugin-only mode works.

## Study before writing the FL bridge (start of L6)

FL Python only runs as a MIDI controller script, no network. External
programs talk to FL via MIDI/SysEx over a virtual port. Study patterns,
don't copy wholesale, check licenses:

- Flapi (github MiguelGuthridge/Flapi, pip flapi): driving FL over MIDI
- fl-studio-api-stubs (pip): every FL API function + docs; install for
  type checks on Linux
- flstudio-mcp (github rosasynthesiz/flstudio-mcp) and fl-mcp-2026:
  loopMIDI setup, which calls work, limits
- PyFLP: .flp read/diff
  Write what you learned + known limits into HANDOFF.md.

## SESSION 1 (Linux) phases

Each phase ends with passing tests + git commit. If time runs out, stop
at a clean phase and write HANDOFF.md anyway.
L1 core: Event, signing, append-only log, BLAKE3 blob store + chunking,
Version events, HostState passthrough, host-neutral plugin identity,
lane kinds + one-mix-lane-per-line invariant, dual-format Audio.
Done: tests write/read/verify/replay a log.
L2 FakeHost + live flow: flit-host trait + capability levels, FakeHost,
settle rule. Done: FakeHost sweeps produce exactly expected events, at
level 1 and level 0.
L3 history: Go back, Main + ideas, per-line lane ownership, Bring in from
idea and other song, undo of a group, Bring-in permission rule.
Done: Bob/Sarah example runs as a test.
Also: Chat events (send, pin to a row, jump to position).
Also: mix ownership handoff; Bring in triggers FLAC fetch.
L4 network: iroh, IDs in keychain (file fallback on Linux), song codes,
request/approve, block, live sync of current line.
Done: two FLit instances on Linux, each on FakeHost, stay in sync.
Also: one at level 1, one at level 0 (mixed crew), still in sync.
Also: chat messages arrive on both instances.
Also: Opus-first then FLAC-in-background transfer, on-demand fetch.
L5 plugin: nih-plug, home screen, Songs/Friends/History/Chat tabs, lane
mode with Opus streaming. Must run with no bridge (level 0).
Build lane streaming so Talk mode and voice can reuse it unchanged
(same encoder, same transport, different flag: in-song vs outside).
Print lanes at the print point. Reference/Local mix toggle (stub the
Reference stream until M3). Full-quality download setting.
Done: loads in a Linux host (Reaper, Bitwig or Carla) with real data.
L6 FL bridge, untested: read references, write bridges/fl (Rust side +
device_FLit.py), PyFLP helpers tested on sample .flp files,
cross-compile Windows build with cargo-xwin.
Done: compiles; every FL assumption marked VERIFY ON WINDOWS; core
crates still have zero FL references (grep for it).
L7 handoff: write HANDOFF.md.

HANDOFF.md must contain:

- What was built per phase and what its tests prove.
- Every VERIFY ON WINDOWS item with file:line and what to do if wrong.
- Exact commands: build on Windows, run tests, install plugin + bridge.
- Lessons + limits from the reference repos.
- Anything half-done and where it stopped.
- How to add a new bridge (what to implement, which tests to copy).
- Which hosts can honor the print point.

## SESSION 2 (Windows) phases

W1 orient: read HANDOFF.md + this spec, install Rust, build natively,
run all tests. Done: all Linux tests pass on Windows.
W2 connect FL: install loopMIDI (dev only), copy device_FLit.py to FL
hardware scripts folder, plugin to VST3 folder.
Done: FL lists FLit as controller + plugin, panel goes green.
W3 verify: run checks below in FL, record results in HANDOFF.md.
W4 solo in FL: swap FakeHost -> FL bridge. Knobs, save, go back, idea,
bring in, all in real FL. Done: M1 usable.
W5 two machines: Windows FLit + real FL, Linux FLit still on FakeHost as
second crew member. Done: edits cross both ways. M2 works.

## Milestones

M0 tonight: log settings/tempo, snapshot project on save, restore list
M1 solo time machine: panel, History, preview, Go back, ideas
M2 two people: IDs, friends, song codes, approve, live setting sync,
text chat
M3 live audio lanes: Opus streaming, auto choose settings vs audio
M3b talk: voice chat (OS audio), Talk mode on any mixer track, record a
received stream into a lane, lesson low-latency settings
M4 saved audio: render changed lanes, chunk, hash, send
-> after M4, any DAW that loads the plugin can join at level 0
(e.g. a friend on Ableton on a Mac: needs the macOS build)
M5 crews 3-10: gossip, lane handoff, owners, remove/block, Bring in
M6 release: signed installers Win+Mac, checklist, ID backup/restore
M7 recipes M8 Door A M9 full focus M10 Door B M11 partners
M12 Ableton bridge (levels 1-3) M13+ more bridges, same trait

## Verify on Windows (decides design)

- Which edits FL's script API reports live vs only in saved .flp
- Can the script set params on third-party VSTs
- Do FL param ids match the plugin's own VST3/CLAP param ids (needed for
  cross-host settings)
- SysEx throughput over loopback MIDI in a busy session
- Plugin copies sharing state in one process (if bridged, use local socket)
- PyFLP on FL 2025/2026 .flp files, read and write
- Same plugin + version + settings = same sound across machines
- Quiet way to render a single lane (bridge vs plugin self-capture)
- FL undo history (general.getUndoHistoryCount etc.) as an act signal
- Can FL print a lane at exactly post-fader, pre-send without rerouting
- iroh direct vs relayed rate (home, hotspot, campus)
- Round-trip delay for Talk mode, direct vs relayed (lesson feel)
- Mic access from inside a plugin process (OS permissions, esp. macOS)
  Later:
- macOS build of the plugin (VST3 + CLAP), AU via clap-wrapper for Logic
- Ableton bridge route: Remote Scripts vs Max for Live
- Reproducible nih-plug builds across machines (else one Build per OS)
- Rust toolchain on producer machines: bundle or fetch on first build
- FL reporting focused window
- Claude Code honoring MCP tools list_changed
- Writing notes from script live vs save-and-reload
- Two DAW instances on one machine (else partners on a second machine)
