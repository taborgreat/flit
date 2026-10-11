# HANDOFF.md

Two sessions so far. **Part A** (top) is session 2: Windows, FL Studio 24.1.1,
2026-10-10, written by the Claude session on Tabor's FL machine. **Part B**
(below it, kept as written) is session 1: Linux, FakeHost, 2026-10-09/10.
Where Part B says `Event`/`Log`/`events.jsonl`, Part A's W6 is the truth
now: the core runs the TreeOS act model (`Stamp`, act chains, fact chains).
Nothing is committed: that is Tabor's job.
Words users see (Studio, member, moment, org, album) and the dead-word list: FLIT_SPEC.md section 0. Code names stay.

# Part A2 — the UPDATE PACK (2026-10-10/11). READ THIS FIRST; it supersedes parts of A0..A11 and Part B

`update.md` (pack 1, sections 0 to 24) was applied across every crate. `FLIT_SPEC.md` is now the spec, with a
table of where each section lives and what is not built. Everything below in this file describing lanes, lane
owners, branches, Bring in, stamps or `.flit/stamps` is history.

## A12.0 State
- `cargo test --workspace`: **94 Rust tests, 0 failed**. Script: `cd bridges/fl/fl-script && python -m unittest
  test_device_flit` (10 tests; do not pass `-I`, it hides the folder).
- **Nothing of the pack has been run inside FL Studio yet.**
- Old songs do not open: a `.flit` folder with `stamps/` is refused. Make the song again.
- The old `identity.key` is adopted as the first Name, unnamed: the panel opens on the first-run walk.

## A12.1 The model in one paragraph
A Name (one key) holds authority; its beings (human, ai, scripted) stamp acts onto their own act chain per
history (`act_seq`). The song's keeper (by default the owner's machine; a `Keeper` act moves it) seals each act
into a fact on the touched thing's chain (`fact_seq`) and the history's seal order (`seal_seq`). Peers apply acts
at once and mark them sealed when the fact arrives. Same property changed by two beings before either saw the
other: first sealed wins, the other is sealed into a new history named after the being and the property. No
clock is read anywhere in ordering (`flit-core/tests/lint.rs`).

## A12.2 Where things are
- Core: `crates/flit-core/src/{act,beings,crew,state,names,clock,song}.rs`, `seal/{mod,verify,fold,keeper,spine,lens}.rs`.
  Disk: `.flit/format`, `.flit/spines/<name-hex>.tree`, `.flit/spines/keeper.tree`, `.flit/blobs`, `.flit/sidecar`.
- Host: `crates/flit-host/src/bridge.rs` (trait, `HostModel`, `diff`, presence), `live.rs` (settle, presence
  settle, `Receiver::apply_state` = diff or snapshot path).
- Net: `SyncMsg::{Hello{heads}, Act, Moment, Want, After}`; `Node::publish()` sends what `Song::take_outbox()` queued.
- Agent: Names under `<data>/names/<hex>/{secret,profile.json}`; keeper claimed when the song names this Name
  and machine (`<data>/machine`); presence pushed every 500 ms; `updates.rs`.
- IPC: protocol 2 (`crates/flit-ipc/src/proto.rs`). A plugin built before the pack cannot talk to this agent.
- Plugin: `theme.rs` (tokens, Plex fonts in `assets/fonts`), `ui.rs` (top bar, rail, Timeline bar, status strip,
  Merge dialog), `ui/tutorial.rs` (first run and Help; written by a second session, ported to the new backend),
  `ui/help.rs` (concept list, second tab of Help).
- FL: tracks are `fl-track-N` (0 = master) and `fl-channel-N`; the `mix` id is gone. Script gained `focus`
  events and `set_track_color`, `set_channel_color`, `select_track`, `select_channel`, `show_window`.

## A12.3 VERIFY ON WINDOWS, new with the pack (none checked yet)
1. `mixer.setTrackColor` / `channels.setChannelColor`: colour format (0xRRGGBB assumed) and whether tinting
   flips FL's "changed" flag (it would look like an edit and could mask a real save). If it does, turn the
   setting off by default.
2. A save started inside FL (Ctrl+S) while tracks are tinted writes the tint into the `.flp`. Only saves asked
   for from the panel restore colours first.
3. `mixer.trackNumber`, `channels.selectedChannel`, `mixer.setTrackNumber`, `channels.selectOneChannel`,
   `ui.showWindow`: exist and behave on API v37.
4. The focus poll in `OnIdle` (~200 ms): cost on a big project.
5. Show in DAW (click a moment on the tape): the read-only look, and coming back with Live.
6. Pack section 20: FL cannot add/remove plugins or restore state chunks from a script (known); cost of many
   applies in a row.
7. Pack section 16: printing a track post-fader pre-send without rerouting.

## A12.4 Not built (also listed in FLIT_SPEC.md)
Listen scrub and the Listen buttons; activity view and its signals; waveform/curve overlays and "Keep both" in
Merge; QR pairing for a delegated being; OS keychain for Name keys; icon set and UI scale; signed releases and
side-by-side installs; host-neutral track ids. PACK 2 (orgs, albums, shelf, standalone app) is untouched.

## A12.5 Build and install
Same commands as A5. `tools/install/install-windows.ps1` builds release (flit-app, flit-agent, fake-session, the
plugin), bundles the plugin and copies app, agent, CLI, script and per-user plugin. The system VST3/CLAP copy needs
`-SystemPlugins` (one UAC prompt) with FL closed; the running helper must be stopped first (it locks its exe).

## A12.6 PACK 2 (2026-10-11), see FLIT_SPEC.md "PACK 2 as built"
- `crates/flit-ui`: theme + fonts + widgets, `client.rs` (Shared and the IPC pump), `host.rs`, `panel.rs` and
  `panel/{tutorial,help,library}.rs`. Both the app window and the plugin draw it.
- `crates/flit-app`: FLit the app. Helper on a tokio thread + eframe window as an IPC client of its own helper
  (`ClientKind::Window`). `--background` (window on demand), `--headless`. Second launch -> `Request::OpenWindow`
  -> the window comes forward (native ShowWindow: a hidden eframe window gets no repaints). Close = hide; the
  tray icon (`tray.rs`: Open FLit / Quit FLit, left click opens) and the plugin's [Open FLit] bring it back.
- IPC protocol 4. `Request::OpenWindow { song, spawn }`, `Push::OpenWindow`, `Push::Invite { kind }`, the Library
  requests and rows (`LibraryView`, `OrgRow`, `OrgMemberRow`, `AlbumRow`, `InviteRow`). Status/SongStatus unchanged.
- Core: `org.rs`; containers of kind Song | Org | Personal share one `Song` type and one `Chains`; the kind is in
  the genesis act's `property`. Net: `Node.containers` beside the open song; sessions are per (peer, container).
- Agent: orgs under `<data>/orgs/<hex>/`, the personal shelf under `<data>/names/<hex>/personal/`, keeper claimed
  per container, `registry.song_notes` (id/org/access per song folder) feeds the Library.
- FL crash on plugin removal / project switch (FLit.clap_unloaded, c0000405): the IPC reader thread outlived the
  DLL. `flit_ipc::Client::drop` now cancels the pipe read (CancelIoEx) and joins; test
  `dropping_a_client_ends_its_reader_thread`.
- Old pre-pack `.flit` beside a project: moved aside to `.flit-before-spines` by `Song::create`; not a song for
  `Song::exists`; auto-link says a failure once.
- Compact plugin (8.5): `flit-ui/src/compact.rs`; four modes (Track send/receive, Talk send/receive). An
  instance named "master" is the session link: Track "master" sends the reference mix (`__reference`) and
  transport; Receiver "master" hears the reference mix in place of the local mix (`Shared.master`, read by the
  audio thread). Any Track instance reports transport; the helper stamps a change once. [Open FLit] =
  `Request::OpenWindow { spawn: true }`. The compact panel never switches songs: it shows the DAW's current
  project and offers "Make it a song" when that project is not one. VERIFY IN FL: typing in the compact panel
  (keyboard workaround), the window size (520x230), [Open FLit] bringing the app forward from inside FL.
- The app opens the DAW: `Request::OpenInDaw { dir }` opens the song and its project file with the OS default
  app (FL for .flp); without a project it starts the newest `Program Files\Image-Line\FL Studio *\FL64.exe`
  (`FLIT_DAW` overrides). Library: "Open in FL Studio" per song, "Create and open FL Studio" on New song.
  VERIFY IN FL: FL already running when a second .flp is opened (FL's own prompt), the .flp association.

# Part A — session 2 (Windows, FL Studio 24.1.1.4239)

## A0. Where things stand (read this first)

Works on this machine today:
- Native build and all tests on Windows (W1): **79 Rust tests, 0 failed** (`cargo test --workspace`), 12 Python tests.
- The core is on the act model (plan.md addendum 3) with the timeline merge order, fold-to-row and checkpoints
  (addendum 4). W6 below. Seven new tests cover the addenda's required cases.
- FL Studio loads the FLit plugin (VST3 and CLAP, both scanned into FL's plugin database), the plugin starts
  `flit-agent` by itself, the agent picks the FL bridge automatically (loopMIDI ports present), reads `.flp` files
  at level 2 (pyflp venv found), logs to `%LOCALAPPDATA%\flit\agent.log`, and the panel in FL was used: FL tempo
  changes landed as one stamp each (140, 148, 136) and "Try an idea" made an idea line.
- The agent's SysEx reaches the loopMIDI port (seen with `scratchpad/midi_probe.py`).

Later in the session the FL script link came up (see A9): the output port had been enabled on the wrong device;
the agent now listens on both ports and the registry mapping was corrected. With the link up FL confirmed: scripting
API v37, Python 3.12.1, `trackCount()` 127, knob moves on a mixer plugin become Setting stamps (baseline first),
tempo and Go back stamp, FL's undo history is visible, `ui.getProgTitle()` is just "FL Studio 2024".

Still open at the time of writing: Listen scrub (audio while dragging); the system VST3/CLAP copies only refresh at
the next FL start; notes need a saved `.flp` (Save version now asks FL to save, see A9).

## A1. The machine and the toolchain (no admin)

- No admin shell, no MSVC, no CMake, no Rust. Python 3.12 and chocolatey present. FL Studio 2024 at
  `C:\Program Files\Image-Line\FL Studio 2024` (24.1.1.4239, embedded Python 3.12 per its `Shared\Python`).
- Rust: `rustup-init.exe -y --default-host x86_64-pc-windows-gnu` (per-user). GNU target because MSVC Build Tools
  need an elevated install; it builds everything including the VST3/CLAP.
- C compiler: portable WinLibs GCC 16.2 (UCRT, posix) unzipped to `%LOCALAPPDATA%\flit-tools\mingw64`.
- CMake (opusic-sys builds libopus): `pip install cmake` into the repo `.venv` (which also holds pyflp and
  fl-studio-api-stubs).
- Every shell needs, before cargo (Git Bash):
  `export PATH="$HOME/.cargo/bin:/c/Users/Tabor/AppData/Local/flit-tools/mingw64/bin:$PATH"`
  `tools/install/install-windows.ps1` sets this up itself.
- loopMIDI: `winget install TobiasErichsen.loopMIDI` (UAC). Ports are created without the GUI: DWORD values named
  after the ports under `HKCU\Software\Tobias Erichsen\loopMIDI\Ports`, then restart `loopMIDI.exe` (the
  installer does this). loopMIDI's own `SysexSize` value there is 256; what that limits is not yet measured.
- Plugins: `C:\Program Files\Common Files\VST3` needs elevation. The installer copies per-user to
  `%LOCALAPPDATA%\Programs\Common\VST3` and `\CLAP`, and with `-SystemPlugins` elevates once for the system
  folders (that is where FL looks by default; FL scanned FLit from there).

Build fixes for the GNU target (all in the repo):
- nih-plug's `assert_process_allocs` is a compile error on windows-gnu: enabled only for
  `cfg(not(all(windows, target_env = "gnu")))` (`crates/flit-plugin/Cargo.toml`).
- iroh declares a `cdylib`; in a debug build GNU ld auto-exports every symbol and overflows the 65535-ordinal
  limit ("export ordinal too large"). `.cargo/config.toml` passes `-Wl,--exclude-all-symbols` for the gnu target;
  the plugin's entry points still export (rustc's .def file).
- IPC: a path-style socket name becomes a named-pipe name on Windows (`flit-ipc/src/server.rs::to_name`).
- Debug-build crypto: unoptimized ed25519/BLAKE3 cost ~9 ms per stamp; `[profile.dev.package.*]` in the workspace
  `Cargo.toml` optimizes just those crates (1200 in-memory stamps: 10.7 s -> 0.1 s).

## A2. W2: connecting FL (what is in place, what FL stores)

- Script: `Documents\Image-Line\FL Studio\Settings\Hardware\FLit\device_FLit.py` (copied by the installer).
- Ports: loopMIDI "FLit In" (agent -> FL) and "FLit Out" (FL -> agent).
- FL stores MIDI device settings in the registry and reads them at start:
  `HKCU\Software\Image-Line\FL Studio 24\Devices\MIDI input\<port>` with `Enabled`, `Port`, `Type`,
  `ScriptFolder` (= the script folder name, "FLit"), `IDString`, `ConnectionCounter`; and
  `...\Devices\MIDI output\<port>` with `Port`, `Sync`, `IDString`. On this box after the user's setup:
  input "FLit In" Enabled=1 Port=236 Type=-3 ScriptFolder=FLit; output "FLit In" Port=236; output "FLit Out"
  Port=-1 (not enabled). While FL was closed, session 2 set output "FLit Out" Port=236 and output "FLit In"
  Port=-1 in the registry (the mapping the script needs); FL reads it at start. The installer could pre-write
  these the same way (FL must be closed); not done yet.
- FL's plugin database lists FLit under `Presets\Plugin database\Installed\Effects\VST3\FLit.nfo` and
  `...\CLAP\FLit.nfo` (`ps_file_idstring=com.flit.plugin`), i.e. the scan found both formats.
- Agent lifecycle: the plugin spawns `%LOCALAPPDATA%\flit\bin\flit-agent.exe` with no arguments; `--bridge auto`
  (the new default) opens the FL bridge when the loopMIDI ports exist. To replace a running agent's exe while FL
  keeps respawning it: rename the running exe (Windows allows that), copy the new one in, then kill the process.
  To stop the plugin from respawning while debugging, rename the exe to `flit-agent.exe.off` and run
  `target\release\flit-agent.exe --bridge fl` yourself (stderr shows panics; `scratchpad/run_agent_fl.sh` did this).
- Logs: `%LOCALAPPDATA%\flit\agent.log` (previous run in `agent.prev.log`), default filter has
  `flit_bridge_fl=debug`; `RUST_LOG` overrides. `fake-session status` prints a `bridge:` line with the link state
  and what to do; the panel shows "fl bridge" green / "not answering" amber with the same text on hover.

## A3. W3: the verify list, status on FL 24.1.1

| Item | Status |
|---|---|
| Project path (title has no path) | **Resolved differently**: FL's recent files in `HKCU\Software\Image-Line\FL Studio 24\MRU` (values "0".."n", newest first) give full paths; the bridge matches the title's `name.flp` against them, then `--projects-dir`, then the default folder. Window title format itself not yet seen (script silent). |
| Script auto-assignment (`# supportedDevices=FLit In`) | FL assigned ScriptFolder=FLit on "FLit In" (registry) after the user chose it; whether it auto-picked is unknown. |
| Output routing by port number | FL's registry confirms the model (input Port and output Port are the same number); the user's mapping put the output on the wrong device; the agent now listens on both ports. |
| SysEx size over loopMIDI (600-byte payload) | Not measured yet (script silent). Agent -> port verified for a 51-byte message. |
| `OnSysEx` vs `OnMidiMsg`, `OnIdle` cadence, dirty flags, `getCurrentTempo` scale, `trackCount` utility track, `muteTrack`, param set on 3rd-party VST3, REC ids, undo-history detail | Open: all need the script link. The heartbeat now carries `changed`, `title`, `undo_count`, `undo_pos` so the first connected minute answers several at once (read `agent.log`). |
| PyFLP on FL 2024 files | **Yes**: the user's `10-9-2025` backup (FL 24.1.1.4239) parses: 22 channels, 21 patterns, 142 notes, 9 inserts, 0 warnings. Native plugin names come back as `_PluginBase` (PyFLP has no parser for them; the slot's internal name would be better). |
| FL tempo into stamps | **Yes**, through the Master plugin's transport info (level-0 path): three tempo changes = three stamps, settled. |
| Keychain identity | **Yes**: same ID across agent restarts (Windows Credential Manager via `keyring`). |

## A4. W4 status
- Panel in FL: tempo stamps and "Try an idea" worked; "Close" was used. Go back / Undo / Bring in / switch line
  push the line's sound into the host (see A6) but could not be exercised against FL knobs yet.
- Save: FL saves are detected from the heartbeat's `changed` flag (1 -> 0 on a freshly written file) plus the MRU
  path; a project load/switch makes a first Version per host. Untested live (script silent).

## A5. Exact commands (Windows)
```
# once per shell (Git Bash):
export PATH="$HOME/.cargo/bin:/c/Users/Tabor/AppData/Local/flit-tools/mingw64/bin:$PATH"
cargo build --release -p flit-agent -p fake-session && cargo xtask bundle flit-plugin --release
FLIT_TEST_PYTHON="$PWD/.venv/Scripts/python.exe" cargo test --workspace
.venv/Scripts/python -I bridges/fl/fl-script/test_device_flit.py ; .venv/Scripts/python -I bridges/fl/flp/test_flit_flp.py
powershell -ExecutionPolicy Bypass -File tools\install\install-windows.ps1 [-NoBuild] [-SystemPlugins]
# agent by hand with full bridge logging:
RUST_LOG="flit_agent=debug,flit_bridge_fl=debug" %LOCALAPPDATA%\flit\bin\flit-agent.exe --bridge fl --name tabor
# CLI (installed next to the agent):
fake-session status | new "Jam" <dir> | open <dir> | history | lanes | show <stamp id> | show | jump <beats> | save "name"
# loopMIDI path probe without the agent:
python -I scratchpad/midi_probe.py 5            # sends a hello into "FLit In", listens on "FLit Out"
python -I scratchpad/midi_probe.py 6 "FLit In" nosend   # watches what the agent sends
```

## A6. What changed in the agent, bridge and panel (session 2)
- `flit-agent --bridge auto` (default): FL bridge when its ports exist, else plugin-only. `--python`,
  `--projects-dir`. `FlConfig.python` is found automatically (`FLIT_PYTHON`, `%LOCALAPPDATA%\flit\venv`, a venv
  next to the exe, the repo `.venv`, `python` on PATH; it must import pyflp).
- The DAW follows History: after Go back / Undo / Bring in / Try an idea / switching lines, and when a peer's
  Go back or Undo arrives, the agent pushes the line's sound into the host (`Receiver::apply_state`: every setting
  the host can set, every fader the song recorded, the tempo) and re-primes the settle rule. Go back also writes
  the project file of this host's nearest Version to `.flit/restore/` and says so (no DAW loads a project through
  a bridge).
- Baselines: the first change of a control in a session is preceded by `HostEvent::Baseline` (its value before);
  the core stamps that first when the song has no value for it, so Go back can put a knob where it was.
  `MixerSet` in `LaneState` remembers which fader fields were ever recorded, so restoring never resets an
  untouched fader. FakeHost sends no baselines (its exact-count tests stay).
- FL bridge: saves from the `changed` flag; project switches from the title's file name; `ProjectOpened` makes a
  first Version only if the host has none; undo-history changes are a dirty hint; hello is retried every 2 s while
  the script is silent; the agent polls the bridge even with no song open; `describe()`/`link_up()` feed the panel
  and CLI; listens on both loopMIDI ports.
- Panel: prefilled "Share this project" and folders from the bridge's current project; chat "jump" moves the host
  transport (`Request::Jump`); History is a timeline (addendum 4): lenses (Name / thing / kinds), a scrubber whose
  release shows the Exact view in the DAW (`Request::Preview { at }`), "Back to now", "who did this" opens that
  Name's lens; bridge state in the top bar and settings.
- Installer rewritten (A1).

## A7. W6: the act model in the core (plan.md addenda 3 and 4)

Vocabulary in code: `Stamp` (`crates/flit-core/src/stamp.rs`), `Thing::{Song, Lane}`, act chain
(`Chains::acts(name, line)`), fact chain (`Chains::facts(thing, line)`), gate (`Stamp::gate()`, true for
`Crew::Remove`). `Id` stays the type of a Name's public key (every crate and the UI's "ID" use it).

- `crates/flit-core/src/seal/` is the small interface: `hash.rs` (ids, folder names), `verify.rs` (`may`: act_prev
  continues the Name's act chain, fact_prev continues the thing's chain, crew/owner MAY check, ranges), `fold.rs`
  (merge order, effective stamps, fold to a row, checkpoints), `stamp.rs` (`Draft` -> sign once -> append to both
  chains), `store.rs` (disk layout), `mod.rs` (`Chains`: in-memory index of both chains + folded line states).
  Swap target for the TreeOS factum crate: everything else only calls `Chains` and `seal::stamp::stamp`.
- Stamp = body { name, act_prev, thing, fact_prev, line, gate, host, wall, payload } + id (BLAKE3 of the body) + one
  signature over the id. `seq`/`parent` are gone. Lane fact chains are linear (fact_prev = exactly the head); the
  song chain may fork (fact_prev = every head the Name saw; the next stamp lists all of them). The line an idea's
  stamps live on is an implicit dependency (`Stamp::deps()`): a stamp is held until the Idea stamp that created
  its line is here.
- Merge order (addendum 4 §2): `fold::merge_order` = topological over act_prev / fact_prev / line, ties by (song
  position, wall clock, id). Used for the per-line fold order, `Chains::causal()`, disk loading and the sync
  layer's send order. Checkpoints every 500 stamps per line (`CHECKPOINT_EVERY`), rebuilt on any non-tail
  insert; `state_at(row)` folds from the nearest safe checkpoint (an Undo reaching back past it disables it).
- Which chain: lane facts (Setting/Mixer/Notes/Audio/HostState), Lane (claim/handoff) and Bring go on the lane's
  fact chain; Crew (Main only), Transport, Version, Idea, Chat, GoBack and Undo on the song chain. GoBack and Undo
  stay line-wide because a non-owner may go back over everyone's lanes; `Undo` is kept as a kind (the addendum's
  kind list omits it, the spec's "one click undoes the group" needs it).
- Disk: `.flit/stamps/<id>` (JSON, written once, tmp+rename), `.flit/chains/acts/<name-hex>/<line-hex>.log`,
  `.flit/chains/facts/<song | lane-<hex of lane id>>/<line-hex>.log`, `.flit/blobs` unchanged. Open loads the
  stamps in merge order (tail appends only). Old `events.jsonl` songs are refused (none existed outside tests).
  Cost: ~2.4 ms per stamp of file I/O on this box (three files touched); batching is a later optimization.
- Tests added (`crates/flit-core/tests`): dual seal on both chains, stored once, song-chain forks and the next
  stamp citing both heads; reject broken act_prev / fact_prev / wrong thing / lying gate; reject a stamp on a lane
  you don't own, also after a handoff; Bring in imports facts not acts (Sarah's act chain on Main does not grow,
  every fact keeps her signature); removed member's act chain stops, facts stay; same order on three peers with
  different arrival orders; fold-to-stamp equals full replay across checkpoints, Undo and Go back; tampered stamp
  file rejected. Fuzz targets renamed (`stamp_decode`, `host_state`), not run (nightly).

## A8. Half-done / next
1. **FL script link** (A0). Once it answers, run the W3 one-liners from `bridges/fl/RESEARCH.md §7` and read
   `agent.log` for api/FL/Python versions, title format, `changed`, undo history, dirty flags per knob move.
2. Listen scrub (addendum 4 §4): decode the Opus chunk of each lane at the scrubbed moment in the agent and push
   it to the Panel plugin as `LaneAudio` on the mix lane while dragging. Exact view exists.
3. Installer: pre-write FL's MIDI device registry values (A2) while FL is closed; copy to the system VST3 folder
   needs FL closed too (the DLL is in use; rename-then-copy works with elevation).
4. Sync Hello still sends every stamp id; chain heads per (thing, line) would be the addendum's "exchange chain
   heads" and is cheap to add in `flit-net/src/sync.rs`.
5. Per-stamp file writes: 3 file operations per stamp; keep chain log handles open or batch.
6. PyFLP helper: report the slot's `internal_name` when the plugin class is `_PluginBase`.
7. `Id` vs `Name`: the addendum's word for an identity is Name; the type stays `Id`. Rename if wanted (touches
   every crate, the IPC and the panel).

## A13. Notes and instruments reach the song (2026-10-11, tutorial session)

Tabor added FL Keys and drew notes; neither showed up. Two causes, both fixed with tests:

- **A channel that arrives with its instrument** (add FL Keys = new channel + generator in one step) only produced
  `TrackSeen`; a Device act came only when a generator was *swapped*. The bridge now announces the generator of a new
  channel and the plugins in the slots of a newly announced mixer track (`fold_channels`, the mixer `is_new` branch).
  Core rule to go with it (`LiveLoop::tick`, `HostEvent::DeviceChanged`): a device is a moment only when it differs
  from what the song has in that slot, so bridges may announce everything they see (first snapshot included) and the
  song dedupes. `bridges/fake/tests/live_flow.rs::announcing_a_device_the_song_knows_is_not_a_moment`.
- **Notes live only in the .flp.** FL's scripting API has no note read (RESEARCH.md); the only path is PyFLP on a
  saved file, and that ran only inside Save version. Project_1.flp had not been written since 10-10 01:56, so nothing
  could land. New, host-neutral: `HostEvent::NotesChanged { path, pos }` = "this file holds the current notes; sync
  them, no Version" -> `LiveLoop::sync_notes` (the same `notes_from_project` that Save version uses: one Notes act
  per clip with added/removed notes). FL bridge: an undo step while the piano roll or playlist is focused, or a stop
  while the record button is on (new heartbeat field `recording`), marks notes dirty; after `FlConfig.notes_settle`
  (3 s) with the transport stopped it asks FL to save (`FlRequest::Save`, tints restored first) and turns the next
  "changed flag back to 0" into `NotesChanged` instead of `ProjectSaved`. Save version / the user's own save still
  make a Version (`version_save_at` wins for 60 s). `FlConfig.quiet_save` (default on) turns it off; untitled
  projects never get a quiet save (FL would open Save As). One notice tells the user the first time.
  `bridges/fl/tests/bridge_flow.rs::notes_edits_make_a_quiet_save_and_notes_changed`.

What this is NOT yet (PACK 3 in update.md): live MIDI as a stream while playing, one Notes act per take from the
stream, scheduled MIDI back into the DAW. The take-as-one-act part already falls out of the file path: FL records a
pass into a pattern, the stop triggers the quiet save, PyFLP reads the pattern, one Notes act per clip.

VERIFY ON WINDOWS (needs FL running the new script + app):
1. `transport.isRecording()` exists on API v37 and reports the record button (else `recording` is False and only
   piano-roll/playlist undo steps trigger the sync).
2. `ui.getFocused(widPianoRoll)` is true while drawing notes (the focus event says `piano_roll`); same for the playlist.
3. `transport.globalTransport(FPT_Save, 1)` from the script saves silently when the project has a file name, and FL's
   `getChangedFlag()` drops to 0 right after (the bridge also checks the file's mtime < 120 s).
4. PyFLP on the FL 24.1.1 file: pattern notes map to `fl-channel-N` by `rack_channel` (`flit_flp.py`), note position
   ticks at the file's PPQ (`flp.rs`). Notes drawn in the step sequencer have length 0.
5. The panel shows one "notes" row per clip per pause, credited to the acting member, and a teammate's copy shows
   it after sync. FL Keys add shows as a "plugin" row on `fl-channel-N`.
Install: `tools\install\install-windows.ps1 -NoBuild` after a release build (the FL script was copied by hand to
`Documents\Image-Line\FL Studio\Settings\Hardware\FLit\device_FLit.py` on 10-11; FL reloads it at start).

## A14. The virtual MIDI port is behind one trait (2026-10-11)

`bridges/fl/src/port.rs` replaces `midi.rs`: `HostPort { open(name), send(bytes), on_receive(cb), close() }` is
the only place platform MIDI code lives; `LoopMidiPort` (midir over winmm, the loopMIDI pair "FLit In" / "FLit
Out" made by hand) is its dev-build implementation; `Wire` turns a `HostPort` into the `MidiPort` the bridge polls.
`FlConfig.port` ("FLit") replaces `in_port` / `out_port`. **loopMIDI is dev-only; it is replaced at M6** (macOS
`CoreMidiPort`, Windows `WinMidiServicesPort` with bundled teVirtualMIDI as fallback: the app creates the "FLit"
port itself; FL's MIDI settings then need one click). FLIT_SPEC.md section 25 has the plan and the VERIFY list.
Nothing outside `port.rs` says loopMIDI any more except the dev installer step (marked DEV ONLY) and the docs.
The messages on the wire are unchanged (`sysex.rs` frames); carrying ACT / MOMENT verbs on it (spec section 21)
is a separate step.

# Part B — session 1 (Linux, FakeHost), as handed to session 2

Read this first, then `plan.md` (the spec) and the two addenda folded into it
(lane kinds + mix lane + Opus/FLAC dual chunks; thin plugin + `flit-agent`
helper process + validation/fuzz/quotas). Everything below was built and
tested on Linux without FL Studio. Nothing here has been committed to git:
that is Tabor's job.

Session 1 ran 2026-10-09 → 2026-10-10 on Linux (Rust 1.96, no sudo).

## 0. Thirty-second tour

```
flit/
  Cargo.toml            workspace (edition 2021), .cargo/config.toml (xtask alias, X11 shim, mingw linker)
  crates/flit-core      events, signing, append-only log, blob store, chunking, versions, lines,
                        ideas, go back, undo, bring in, chat, crew, validation. NO host code.
  crates/flit-host      HostBridge trait, capability levels, settle rule, LiveLoop, Receiver
  crates/flit-audio     Opus + FLAC per-bar chunk codecs, resampler, live Opus streaming (lanes/Talk)
  crates/flit-net       iroh: IDs, song codes, join/approve/invite, sync, blobs, streams, quotas
  crates/flit-ipc       plugin <-> agent protocol (Unix socket / named pipe), blocking client, server
  crates/flit-agent     the helper process: owns song + net + codecs + bridge, serves plugins
  crates/flit-plugin    nih-plug VST3 + CLAP, thin IPC client, egui panel, ring buffers
  bridges/fake          FakeHost: simulated DAW, any level 0-3
  bridges/fl            FL bridge: Rust side, fl-script/device_FLit.py, flp/ (PyFLP helper), RESEARCH.md
  tools/fake-session    CLI client for a running agent (drives demos and the W5 two-machine test)
  tools/install         install-windows.ps1, install-linux.sh
  tools/linux-shim      lets the GUI build on a Linux box without X11 -dev packages
  fuzz/                 cargo-fuzz targets (nightly to run; stable to type-check)
  xtask/                `cargo xtask bundle` (nih-plug bundler)
```

Process model (addendum 2): the DAW loads `FLit.vst3`/`FLit.clap`, which is a
thin client. It starts `flit-agent` if none is running and talks to it over
local IPC. One agent per user per machine serves every plugin instance and
every DAW open at once. The agent holds the song (`.flit/` next to the
project), the network (iroh), the codecs and the bridge. If the agent dies,
audio keeps flowing through the plugin and the panel shows "reconnecting".

## 1. What was built per phase and what the tests prove

Totals at the end of session 1: **73 Rust tests, 0 failed** (`cargo test --workspace`, 32 s; 79 after session 2's act model), plus
**12 Python tests** for the FL script and the PyFLP helper, plus a real CLAP
host run (clap-validator) that loaded the plugin with an agent behind it.

### L1 core — `crates/flit-core` (17 unit + 16 integration tests)
- `Event { id, sig, body }`, `body = { parent, seq, line, author, time, lane, host, payload }`.
  `id = BLAKE3(postcard(body))`, `sig = ed25519(author, id)`. Two fields the
  spec did not list were added: `seq` (parent.seq + 1) and `line`. They make
  the canonical order inside a line `(seq, wall_ms, id)`, so two people
  appending concurrently converge to the same order and state
  (`log_roundtrip.rs::concurrent_writers_converge`).
- Append-only `Log`: validates hash, signature, chain (`BadSeq`, `WrongLine`),
  crew (`NotInCrew`, `NotOwner`, `LastOwner`), lane ownership
  (`NotLaneOwner`), value ranges (`log::check_ranges`: settings 0..1, pan,
  volume, tempo 20..999, notes, audio formats, text sizes, HostState ≤ 4 MB,
  mix-lane invariant). Events whose parent has not arrived are held and
  applied when it does (`out_of_order_arrival_is_held_then_applied`).
  Materialized `LineState` per line is kept incrementally.
- Storage (session 1): `.flit/events.jsonl`. **Session 2 replaced this** with stamps + act/fact chain logs
  (Part A, A7); tampering is still caught on open. `.flit/blobs/<hh>/<hash>` BLAKE3-verified (`BlobStore`), `.flit/song.json`.
- Chunking by bar: `chunk::ChunkPlan`; a small edit re-chunks one bar
  (`small_edit_makes_few_new_chunks`).
- Payloads: Setting (host-neutral plugin identity + param id + 0..1), Mixer,
  Transport, Notes, Audio (`opus_chunks` + `flac_chunks`, `print_point`,
  `bit_depth`, `credit`), Version (host + project hash + manifest), Crew,
  Lane (`kind: Part|Bus|Mix`), Idea, Bring (carries the source events
  verbatim, original authors kept), Chat (pinned row), HostState (opaque
  blob + host tag), GoBack, Undo.
- Mix lane invariant: `Song::create` makes the `mix` lane (kind Mix, owner =
  creator); a second Mix lane or a kind change is refused.
- Private per-user state: `BlockList` (file), `UserSettings` (download
  policy always / on demand / Wi-Fi only).

### L2 FakeHost + live flow — `crates/flit-host`, `bridges/fake` (3 + 6 tests)
- `HostBridge` trait (`flit-host/src/bridge.rs`): `tag`, `capabilities`,
  `poll` (host → core events), `has_plugin`, `set_param`, `set_mixer`,
  `set_transport`, `read_project`, `render_lane`, `capture/apply_host_state`,
  `lanes`, `master_has_processing`. `Capabilities` declares the level and
  per-feature flags (incl. `honors_print_point`).
- Settle rule (`settle.rs`): 150 ms still → one event; knob back to its
  start → zero events (`thousand_twists_one_event`,
  `returned_to_start_zero_events`).
- `LiveLoop` (`live.rs`): polls a bridge, settles, appends signed events,
  auto-claims unowned lanes, drops edits on lanes someone else owns, saves
  versions (project blob + manifest + notes from `read_project` at level 2 +
  renders only lanes changed since this host's last version). Level 0 path:
  `report_transport` (plugin reads tempo/transport from its host) and
  `push_captured` (plugin captures its own lane at the print point).
- `Receiver` decides per incoming event: same plugin + bridge can set params →
  apply; same host → apply HostState; else hear the lane as audio. It only
  ever applies events that the log already accepted (`Receiver::apply`
  returns `None` for unknown ids).
- FakeHost tests prove: level 1 sweep of 1000 twists = exactly one Setting;
  return-to-start = none; level 0 is silent for knobs but transport crosses
  and a save captures 2 bars of audio as 2 chunks; saves render only changed
  lanes; a level-1 and a level-0 FakeHost in one song stay equal
  (`two_fakehosts_level1_and_level0_share_a_song`); mixed-crew receiver
  decisions (`receiver_mixed_crew_decisions`).

### L3 history — `crates/flit-core/tests/history.rs` (6 tests)
- `bob_and_sarah`: the spec's example verbatim. Sarah's idea from the row
  before Bob's change starts at 0.2 while Main is 0.9; Drums on the idea is
  inherited (Bob's) until Sarah takes it; Main stays untouched; she offers it
  (chat pinned to the idea's head, jump target works); she may NOT bring it
  in (not owner on Main); Bob brings it in as one group; one Undo removes the
  group; Undo of the Undo restores it; keep-both lands on a new lane.
- `go_back_restores_sound_and_is_undoable`: `RestorePlan` = nearest Version
  of *that host* + the edits after it (another host without a version replays
  everything); Go back is an event and is undone like any other; Go back can't
  target the future or another line.
- `ideas_are_separate_lines_and_lanes_are_owned_per_line`, nested ideas,
  persistence across reopen.
- `bring_in_from_another_song_keeps_author_and_needs_flac`: the Bring in is
  refused until the FLAC chunks are present (`BlobNotFound`), authors kept,
  Opus still reported missing.
- `mix_lane_handoff_and_bring_on_mix_lane`: handoff by Lane event, idea
  inherits the mix owner, Bring in on the mix lane follows the normal owner
  rule, keep-both on a mix lane lands as a Part.
- `chat_send_pin_jump_and_idea_tagging`: chat rows sit beside edits, pinned
  jump goes to the row's moment, idea chat is tagged with the idea's name.
- `validation.rs` (addendum 2 §3): wrong author, wrong lane, out-of-range
  values, bad chain, bad signature, second genesis, second mix lane, removed
  member, unknown kind — all rejected, never applied.

### L4 network — `crates/flit-net` (3 unit + 3 integration tests), `crates/flit-agent` (2)
- `Node::bind`: iroh 1.3 endpoint; ID = ed25519 key = iroh endpoint id
  (`IdentityStore`: OS keychain on Windows/macOS via `keyring`, 0600 file on
  Linux, backup/restore as hex). `local_only` (tests/LAN: loopback, no relay,
  no discovery) and `relay_only` (privacy: `clear_ip_transports`).
- Song code = `flit1` + base32(song id, owner id, owner address). Friend
  code = `flitid1` + base32(id + addresses); a bare ID also parses.
- Door 1 `request_join` → owner gets `NetEvent::JoinRequest` → `approve`
  appends the Crew event and answers `Welcome{genesis, name, peers}`; `deny`,
  `block` (refused at the door and in sync). Door 2 `invite` → friend gets
  `NetEvent::Invite` → `accept_invite`.
- Sync (`sync.rs`): symmetric Hello with all event ids, each side sends what
  the other lacks, then live push; missing parents are asked for (`Want`);
  strangers and blocked IDs are refused before Hello; a forged event from a
  crew member is rejected and reported (`Rejected{reason}`), never applied.
- Blobs (`blobs.rs`): `Get(hashes)` → `Found{len}`+bytes / `NotFound`,
  verified on receipt, counted against quota. After importing Audio events
  the node fetches **Opus first, then FLAC in the background** per the
  download policy; `fetch_missing(line, force_flac)` for on-demand.
- Streams (`ALPN flit/stream/1`): control over a bi stream, Opus frames as
  QUIC datagrams.
- Quotas (`quota.rs`): per-peer token bucket (events/s, burst) and stored
  bytes; over quota → stop accepting, `QuotaExceeded`, shown in the panel.
- `two_nodes.rs::join_sync_chat_and_blobs`: two instances, Alice level 1
  FakeHost, Bob level 0; join by code; one sweep → one Setting at Bob; Bob
  hears drums as audio; chat both ways incl. pinned jump; Bob's level-0 save
  produces real Opus+FLAC chunks; Alice fetches **Opus before FLAC** and
  decodes the take losslessly; both `LineState`s equal.
  `strangers_and_blocked_ids_are_refused`, `door_two_invite_and_quota`.
- `two_agents.rs`: the whole stack through IPC: two agents in one process,
  join by code via `fake-session`-style clients, FakeHost sweep → history on
  the other agent, chat push to a Panel client, a level-0 Lane client streams
  live audio that a Receiver client on the other agent hears, save → Opus+FLAC
  fetched, Try an idea / Switch line replicate. `plugin_client_reconnects_after_agent_restart`.

### L5 plugin — `crates/flit-plugin` (4 tests + clap-validator)
- nih-plug (git rev `de42101`), VST3 + CLAP, egui panel 760×520.
  Modes: Master (panel), Lane (send), Lane (receive), Talk (send), Talk
  (receive). Lane name is a persisted field. `receive_gain`, `monitor`,
  `hear_reference` params.
- `engine.rs`: the audio callback only pushes/pops two rtrb rings; tests
  prove passthrough, capture, non-blocking overflow when no agent, receiver
  mixing, and "Hear: Reference mix" replacing output only when connected.
- `worker.rs`: one std thread owns the `flit_ipc::Client`; `connect_or_start`
  (env `FLIT_AGENT`, then `%LOCALAPPDATA%\flit\bin\flit-agent.exe`, then
  PATH); reconnect with backoff; sends Audio blocks (with host position) and
  Transport for level 0; routes pushes to the rings; runs panel actions.
- `ui.rs`: home (ID + QR + New song / Share this project / Join with a song
  code), tabs Songs / Friends (join requests Approve/Deny/Block, invites,
  crew, friends by code) / History (lines, lanes, Bring in with preview,
  conflict + keep-both, Offer, rows with Go back / Try an idea / Undo / Pin)
  / Chat; settings (Hear reference/local, download policy, monitoring kbps
  64–256, submix, relay-only, display name); Talk button present but greyed
  (voice is M3b). Plain words only.
- Linux host check: `clap-validator validate target/bundled/FLit.clap` with
  an agent running: **30 passed, 4 failed, 10 skipped**. The plugin loaded,
  processed audio, connected to the agent and the host's transport info
  showed up as a Transport event in the song (real data). The 4 failures are
  inside nih-plug's CLAP wrapper, not FLit code — see §6.
- Bundles: `target/bundled/FLit.clap`, `FLit.vst3` (Linux `.so`, Windows
  `.vst3` after the cross build).

### L6 FL bridge (untested on FL) — `bridges/fl` (4 unit + 4 integration Rust tests, 8 + 4 Python tests)
- `RESEARCH.md`: full notes on Flapi, fl-studio-api-stubs, flstudio-mcp and
  siblings, FL scripting facts, PyFLP (licenses, limits, VERIFY list).
- `sysex.rs`: `F0 7D 'F' 'L' 'i' 't' dir id_hi id_lo cont base64(json) F7`,
  600-byte payload chunks, reassembly, `FlRequest`/`FlReply`/`FlEvent`.
  `python_codec_matches_rust` checks byte-for-byte against vectors the Python
  test writes (`fl-script/codec_vectors.json`).
- `bridge.rs`: `FlBridge: HostBridge` at level 1 (level 2 when a Python with
  pyflp is configured). Hello → Snapshot → paginated `Params` scans (first
  scan = baseline), `dirty` events → re-read → `ParamChanged` only for real
  changes, echo suppression for settings we applied, mixer volume law
  0.8 = unity, heartbeat → Play/Stop/Tempo, focus, project path guessed from
  the window title. `bridge_flow.rs` runs it against a scripted FL stand-in.
- `midi.rs`: `MidiPort` trait, `LoopbackPort` (tests), `real::MidirPort`
  (Windows only, midir over winmm, ports "FLit In"/"FLit Out").
- `fl-script/device_FLit.py`: the controller script (`# name=FLit`,
  `# supportedDevices=FLit In`), thin handlers, heartbeat from `OnIdle`,
  `OnRefresh`/`OnDirtyMixerTrack` → `dirty`, both `OnSysEx` and `OnMidiMsg`,
  exceptions never escape. `test_device_flit.py` runs it with fake FL modules.
- `flp/flit_flp.py` + `flp.rs`: PyFLP (GPL-3, separate process, never linked)
  → JSON → `ProjectSnapshot` (notes per mixer insert). Tested on the two
  sample files (`flp/samples`, FL 20.8.4): tempo, 19 channels, 48 notes, the
  OTT VST slot, byte-identical save round-trip.
- Cross-compile: `cargo build --release --target x86_64-pc-windows-gnu -p
  flit-agent -p flit-plugin -p fake-session` and `cargo xtask bundle
  flit-plugin --release --target x86_64-pc-windows-gnu` both succeed on Linux
  with mingw (`flit-agent.exe` 19 MB, `flit_plugin.dll` 8 MB, bundle in
  `target/bundled/FLit.vst3/Contents/x86_64-win/`). Note: both targets write
  `target/bundled/FLit.clap`; bundle one target at a time.
- Core cleanliness: `grep` for FL/flp/loopMIDI/SysEx/Image-Line in
  `crates/flit-core|host|net|audio|ipc|plugin` is empty; no core crate
  depends on a bridge crate (flit-net has a *dev*-dependency on
  `flit-bridge-fake` for its tests only, as the spec's "test against
  FakeHost" asks).

### L7 — this file.

## 2. Every VERIFY ON WINDOWS item (file:line) and what to do if wrong

Session 2 status for these rows is in Part A, A3 (project path: resolved via FL's MRU registry key; port
routing: confirmed, agent listens on both ports; the rest wait for the script link). Line numbers below are
session-1 line numbers and have drifted.

Also run the checklist in `RESEARCH.md §7` first; it has the one-liners to
type into FL's Script output > Interpreter.

| Where | Assumption | If wrong |
|---|---|---|
| `bridges/fl/src/sysex.rs:14`, `fl-script/device_FLit.py:25` | 600-byte base64 payload per SysEx survives loopMIDI→FL→loopMIDI | Lower `CHUNK_PAYLOAD` in both files (they must match). Raise toward 900 if it survives. |
| `fl-script/device_FLit.py:106` | `device.midiOutSysex` needs F0/F7 in the bytes and routes to the output port with the same Port number | Check *Options > MIDI Settings* port numbers; disable "FLit In" as output and "FLit Out" as input. |
| `fl-script/device_FLit.py:297`, `:302` | FL 21+/API v40 delivers SysEx to `OnSysEx`; older builds to `OnMidiMsg` | Both are implemented; if neither fires, log `event.status` in `OnMidiIn` and route from there. |
| `fl-script/device_FLit.py:308` | `OnIdle` fires ~every 20 ms (heartbeat every 25 calls) | If it never fires (one report on 25.2.5), send heartbeats from `OnRefresh`/`OnUpdateBeatIndicator`, and have the agent poll `Snapshot` every 500 ms (set a timer in `FlBridge::pump_queue`). |
| `fl-script/device_FLit.py:325`, `bridges/fl/src/bridge.rs:233` | `OnRefresh` flags 4096 (ControlValues) / 4 (Mixer_Controls) / 512 fire for VST knob moves and fader moves | Log `flags` for a knob move and a fader move; add the observed flags to `interesting`. Fallback: the bridge re-scans dirty tracks on every heartbeat. |
| `fl-script/device_FLit.py:127` | `mixer.getCurrentTempo()` returns BPM×1000 | The code divides when > 1000; if FL returns BPM already, that branch is skipped. Confirm with `mixer.getCurrentTempo()` in the Interpreter. |
| `fl-script/device_FLit.py:245` | Tempo set via `processRECEvent(REC_Tempo, bpm*1000, UpdateValue|UpdateControl)`; adding `REC_FromMIDI` breaks it | If the stubs' v38 `mixer.setCurrentTempo` exists on your build, prefer it. |
| `fl-script/device_FLit.py:133` | `transport.getSongPos(SONGLENGTH_ABSTICKS)` = absolute ticks in song mode | Falls back to `mixer.getSongTickPos()`. Check pattern mode. |
| `fl-script/device_FLit.py:156`, `bridges/fl/src/bridge.rs:14,376` | Mixer fader 0.8 = unity (0 dB); linear in the normalized scale | Measure `mixer.getTrackVolume(i, 1)` (dB) at a few fader positions and replace `fl_to_gain`/`gain_to_fl`. |
| `fl-script/device_FLit.py:165` | `mixer.trackCount()` includes a trailing utility track | If not, change `range(0, max(1, n - 1))` to `range(n)`. |
| `fl-script/device_FLit.py:193` | `ui.getFocusedFormID()` = track·4194304 + slot·65536 for effects | Only affects Focus (level 3, not needed for M1/M2). |
| `fl-script/device_FLit.py:230` | `plugins.setParamValue` on a third-party VST3 effect updates its GUI | Try `pickupMode=2`; if the GUI lags, call `general.processRECEvent` on the REC id (RESEARCH §2.3) instead. |
| `fl-script/device_FLit.py:242` | `muteTrack(t)` bare toggle works, `muteTrack(t, 1)` does not | Flip the branch if your build honours the value argument. |
| `fl-script/device_FLit.py:263`, `bridges/fl/src/bridge.rs:29,49,367` | `ui.getProgTitle()` contains "name.flp"; projects live in `Documents\Image-Line\FL Studio\Projects` | If the title has no file name: read FL's recent-files registry key or ask the user to pick the project folder once (agent `--open`). Level 2 and Version events depend on this. |
| `bridges/fl/src/bridge.rs:22` | Named VST params live below index 1024 | Raise `MAX_SCAN_PARAM` if a plugin reports named params above it (the API reports 4240). |
| `bridges/fl/src/bridge.rs:157` | Lane id `fl-track-<n>` is stable (FL has no track UUIDs) | If users reorder tracks, key lanes by track *name* instead (`lane_for_track`). |
| `bridges/fl/src/bridge.rs:399` | No quiet single-lane render through the API: the FLit plugin on the track captures itself | If FL offers a render path (e.g. a pyscript), implement `render_lane`. |
| `bridges/fl/src/bridge.rs:404`, `tests/bridge_flow.rs:103` | FL cannot print post-fader/pre-send without the user rerouting (`honors_print_point: false`) | If the plugin can sit post-fader (FL's "post" slot?) set it true and record it in §7 of the spec. Otherwise scale the captured audio by the fader in the bridge. |
| `bridges/fl/src/bridge.rs:437` | FL exposes only the plugin *name*, so settings share only between FL users (`PluginFormat::Other("fl-wrapper")`) | If FL exposes the real VST3/CLAP param ids and vendor/version (spec verify item 3), build a proper `PluginIdentity` and settings will cross hosts. |
| `bridges/fl/src/midi.rs:69` | winmm delivers one complete SysEx per midir callback | If messages arrive fragmented, accumulate until 0xF7 in the callback. |
| `bridges/fl/flp/flit_flp.py:13` | PyFLP parses your FL 2025/2026 `.flp` | Expect playlist warnings (issue #200); channels/patterns/mixer should parse. If not, evaluate `dawhubapp/flpdiff` or write a parser. |
| `crates/flit-plugin/src/worker.rs` `agent_exe()` | `%LOCALAPPDATA%\flit\bin\flit-agent.exe` is where the installer puts the agent | Set `FLIT_AGENT` to the exe path if you keep it elsewhere. |

The spec's own "Verify on Windows (decides design)" list maps to: FL live
vs saved edits → `dirty` flags (above); third-party param set → `:230`;
param ids match VST3 ids → `:437`; SysEx throughput → `:14`; plugin copies
sharing state in one process → not needed (every copy is a thin client of
one agent); PyFLP on new files → `flit_flp.py`; same plugin+settings = same
sound → compare a rendered bar on both machines; quiet single-lane render →
`:399`; iroh direct vs relay rate and Talk round trip → run `fake-session`
two-machine test with `--local` off and watch `peers` + `missing chunks`;
mic access from a plugin → M3b.

## 3. Exact commands

### Build on Windows (native)
```
winget install Rustlang.Rustup   (or rustup-init.exe), then: rustup default stable
winget install Kitware.CMake      (opusic-sys builds libopus with CMake)
cargo build --release -p flit-agent -p fake-session
cargo xtask bundle flit-plugin --release
cargo test --workspace            (W1 goal: everything that passed on Linux passes here)
```
Python side (for the script/helper tests and level 2):
```
py -3.12 -m venv .venv && .venv\Scripts\pip install pyflp fl-studio-api-stubs
.venv\Scripts\python -I bridges\fl\fl-script\test_device_flit.py
.venv\Scripts\python -I bridges\fl\flp\test_flit_flp.py
set FLIT_TEST_PYTHON=%CD%\.venv\Scripts\python.exe && cargo test -p flit-bridge-fl
```
Install plugin + agent + FL script: `powershell -ExecutionPolicy Bypass -File tools\install\install-windows.ps1`
(prints the loopMIDI / MIDI Settings steps). Agent with the FL bridge:
`%LOCALAPPDATA%\flit\bin\flit-agent.exe --bridge fl` (set
`--data-dir`, `--name`; add `--relay-only` for privacy). Level 2 needs the
agent to know a Python with pyflp: wire `FlConfig.python` (not exposed on
the CLI yet; see §5).

### Cross-compile from Linux (what session 1 did)
```
rustup target add x86_64-pc-windows-gnu      (mingw-w64 gcc must be installed)
cargo build --release --target x86_64-pc-windows-gnu -p flit-agent -p flit-plugin -p fake-session
cargo xtask bundle flit-plugin --release --target x86_64-pc-windows-gnu
```
`cargo-xwin` (MSVC target) was not used because mingw was already on the
box; it should also work.

### Run the tests on Linux
```
cargo test --workspace
clap-validator validate target/bundled/FLit.clap     (clap-validator 0.4.1, ~/.local/bin)
```
Linux box notes (no sudo): user-local cmake in `~/.local/opt/cmake`;
`tools/linux-shim` + `.cargo/config.toml` stand in for the X11 -dev packages
(harmless where they are installed).

### Two-machine test (W5)
Windows (real FL):
```
flit-agent --bridge fl --name tabor
fake-session new "Jam" C:\Users\you\Documents\Image-Line\FL Studio\Projects\Jam
fake-session code              -> paste the flit1... code to the Linux side
fake-session status            -> shows the join request; fake-session approve <id>
```
Linux (FakeHost as second crew member):
```
~/.local/share/flit/bin/flit-agent --bridge fake --name linux-fake
fake-session join <code> ~/songs/jam
fake-session fake-track drums Drums && fake-session fake-sweep drums thresh 0 0.8
fake-session history           -> both sides show the Setting; `fake-session watch` prints pushes
fake-session tone vox --seconds 3   (stand-in for a level-0 lane plugin; Windows hears it on a Receiver)
fake-session save "take 1"     -> Windows status shows "missing chunks" going to 0
```
Use `--local` on both agents only when both are on one machine; across
machines leave it off (n0 relay + discovery). Both agents must be reachable:
the song code carries the owner's addresses.

## 4. Lessons and limits from the reference repos (short; RESEARCH.md has all)
- Only a MIDI controller script gets into FL. Sandbox: no sockets, threads,
  or file writes on FL 24/25; everything runs on FL's UI thread; heavy work
  in one tick stalls FL → the script stays thin, pagination everywhere.
- SysEx over Windows MIDI: keep each message under ~1 KB (we use 600-byte
  payloads). Base64 beats 7-bit packing for simplicity (+33 %).
- No per-parameter change callback: dirty flags + re-read. No API to read
  the project path, load plugins/audio, render, or place playlist clips.
- VST wrappers report 4240 params; real ones have names at low indices.
  Values are 0..1 with no unit mapping (`getParamValueString` gives text).
- Mixer unity = 0.8; tempo raw = BPM×1000; `muteTrack(t,1)` unreliable;
  `REC_FromMIDI` rescales values.
- Flapi: MIT, unmaintained, pickle on the wire (don't). flstudio-mcp: MIT,
  best empirical docs for FL 25.2.5. fl-studio-api-stubs: LGPL, v37 (FL ~24),
  `midi.REC_Mixer_*` constants missing from `midi/__init__.py`.
- PyFLP: GPL-3 (kept in its own process), unmaintained, needs a Python 3.12
  monkeypatch (in `flit_flp.py`), playlist structs broken for FL 21+/2025
  files, byte-exact round trip for what it does not understand.
- FL 2026 may ship a built-in MCP/`script_handler` (unverified claim in
  RESEARCH §3.3) — if true, it could replace much of the MIDI bridge.

## 5. Half-done / deviations / where things stopped
- **Not committed.** No `git add`/`commit` was run (per instructions).
  `.gitignore` covers `target/`, `__pycache__/`.
- **iroh-blobs and iroh-gossip are not used.** The spec names them; session
  1 used plain iroh connections with a small verified blob protocol
  (`flit-net/src/blobs.rs`, chunk-sized so it is resumable by nature) to
  avoid a second on-disk blob database next to `.flit/blobs`. Gossip (3+
  peers) is M5; the sync layer already forwards imported events to other
  connected peers, and `connect_crew` dials everyone known. Swap in
  iroh-blobs/gossip behind the same `Node` API if relay-scale transfer or
  large crews need it.
- **Sync Hello sends every event id.** Fine for thousands of events; add a
  per-line `(count, max seq)` summary or a bloom filter for huge songs.
- **Clap-validator**: 4 failures inside nih-plug (§6).
- **Reference mix / Talk**: implemented end to end over IPC + net (master
  plugin streams `ReferenceMix` when it owns the mix lane; receivers with
  "Hear: Reference mix" get it; Talk send/receive and "record a received
  Talk stream into a lane" exist in the agent) but only exercised by unit
  tests and the two-agent test for lanes; not yet heard in a real DAW.
  Voice (OS audio) is M3b, button greyed.
- **Submix** sums blocks as they arrive (no cross-lane alignment).
- **Level 2 on FL** needs `FlConfig.python`; the agent does not expose it on
  the CLI yet (`crates/flit-agent/src/main.rs`, `"fl" =>` branch).
- **Chat "jump to moment"** in the panel shows a notice; moving the host
  transport needs the bridge (`set_transport(Position)` exists).
- **QR code** is drawn from the friend code (long); a shorter code via
  discovery-only (bare ID) is `AddrInfo::decode` compatible.
- **Fuzz targets** type-check on stable; running needs nightly
  (`fuzz/README.md`).
- **Windows keychain** path (`keyring` v1, Credential Manager) compiles for
  the mingw target but was not run.
- **Reaper/Bitwig on Linux** were not available without sudo; the CLAP host
  used was clap-validator. Reaper installs to the home dir if you want a GUI
  check under `xvfb-run`.

## 6. Known issues
- clap-validator `state-invalid` crashes: nih-plug's `ext_state_load` trusts
  the length prefix in the state stream (`Vec::with_capacity(length)`) and
  aborts on 3 MB of random bytes. Real hosts only feed back states the plugin
  wrote. Upstream nih-plug; consider a `[patch]` if it matters.
- clap-validator `state-reproducibility-*`: values restore correctly, but
  nih-plug posts `Task::ParameterValuesChanged` (editor only) rather than
  `RescanParamValues` to the host after a state load. Hosts rescan on their
  own; cosmetic for DAWs. Upstream nih-plug.
- The egui `copied_text` deprecation was replaced by `ctx.copy_text`.

## 7. How to add a new bridge
1. New crate under `bridges/<host>/` depending on `flit-core` and
   `flit-host` only. Pick a `HostTag` string (one word).
2. Implement `HostBridge` (`crates/flit-host/src/bridge.rs`): at least
   `tag`, `capabilities`, `poll`. Level 1: `has_plugin`, `set_param`,
   `set_mixer`, `set_transport`; emit `HostEvent::ParamChanged/MixerChanged/
   Transport/LaneSeen` from `poll`. Level 2: `read_project` →
   `ProjectSnapshot` (notes per lane, files), emit `ProjectSaved`. Level 3:
   `FocusChanged`. Set `honors_print_point` honestly.
3. Build a `PluginIdentity` that is the plugin's real format + vendor +
   name + version so settings cross hosts; use the plugin's own param ids.
4. Copy the tests: `bridges/fake/tests/live_flow.rs` (run your bridge
   through `LiveLoop` and `Receiver`), `bridges/fl/tests/bridge_flow.rs`
   (protocol flow against a stand-in of the host). Mark every unverified
   host fact `// VERIFY ON <OS>` and list it here.
5. Add a variant to `AnyBridge` in `crates/flit-agent/src/bridge.rs` and a
   `--bridge <name>` arm in `main.rs`. The agent is the only crate allowed to
   know every bridge.
6. Ableton note: Remote Script (Python, has sockets) or Max for Live; `.als`
   is gzipped XML, so level 2 is a parser away.

## 8. Reading order for session 2 (W1)
`plan.md` → this file → `crates/flit-host/src/bridge.rs` →
`bridges/fl/src/bridge.rs` + `fl-script/device_FLit.py` →
`crates/flit-agent/src/agent.rs` → `crates/flit-plugin/src/{lib,worker,ui}.rs`.
Then `cargo test --workspace`, then `tools/install/install-windows.ps1`.

## A9. Later in session 2: FL link up, what FL answered, what was added

Link: the user had enabled the script on input "FLit In" (port 236) but the Output list had "FLit In" (236) instead
of "FLit Out". FL routes `midiOutSysex` to the output device with the script's port number, so FL's answers went back
into the agent's own send port. Fix in two places: the agent listens on both loopMIDI ports (`bridges/fl/src/midi.rs`,
either mapping works) and, with FL closed, the registry was set to output "FLit Out" = 236 / "FLit In" = -1. The panel
and `fake-session status` now say "waiting for the FL script" with the exact settings until the first heartbeat.

Verified on FL 24.1.1 build 4239 (through the heartbeat/hello and the song's history):
- `general.getVersion()` = 37 (scripting API), `sys.version` 3.12.1; `OnIdle` fires (heartbeats every ~500 ms arrive).
- `mixer.trackCount()` = 127: Master, 125 inserts, one utility track (`range(n-1)` is right).
- Params replies of 48 params (several KB) travel as 600-byte-payload chunks both ways: the Setting stamps prove it.
- A knob on the master's Fruity Compressor: one Baseline stamp ("Fruity Compressor 0 = 1.000") then the settled
  change (0.728). Tempo changes: one stamp each. Go back: stamps land (knob follow-through to FL not watched yet).
- `ui.getProgTitle()` = "FL Studio 2024" (no file name, saved or not). Project path now falls back to FL's most recent
  file (`MRU` "0"), which pointed at the project the user had just saved (`Projects\Project_1\Project_1.flp`).
- `general.getUndoHistoryCount/Pos` move together per edit ((2,2) -> (3,3) ...); every change is a dirty hint.
- "RuntimeError: Plugin not valid" from `plugins.getParamCount` after a plugin left its slot mid-scan: the scan ends.
- Claiming lanes: the first snapshot had claimed all 126 inserts. Now only tracks in use (name changed, plugins,
  fader/pan/mute off default) are lanes; an unused insert becomes a lane the moment something happens on it.

Added for the user's reports:
- **Plugins added/removed are facts**: `Payload::Device { slot, plugin: Option<PluginIdentity> }` (a lane edit; the
  slot's settings start over when its plugin changes; `LaneState.devices`). The FL bridge diffs the slot lists of
  mixer tracks and the generators of channel-rack channels; FakeHost has `add_device/remove_device`. Receiver
  applies it where a bridge can place plugins (`Capabilities.set_devices`, FakeHost yes, FL no: its API cannot load
  plugins); otherwise `apply_state` returns `manual` steps and the agent tells the user what to put where.
- **Channel-rack channels are lanes** (`fl-channel-<global index>`): their generator is device slot 0 (params via FL
  slot -1 with `useGlobalIndex`), their own volume/pan/mute are the lane's fader (0.78125 = unity). PyFLP notes map to
  the same lanes, so notes read on save and live knob moves of an instrument meet on one lane.
- **Save version asks FL to save** (`transport.globalTransport(FPT_Save, 1)` via `FlRequest::Save`,
  `HostBridge::request_save`): the Version (with the real `.flp`, and at level 2 the notes) lands when the heartbeat
  reports the written file; the name typed in the panel is used (`LiveLoop::next_version_name`). An untitled project
  gets FL's Save As dialog.
- **Typing in the panel inside FL**: egui-baseview's `windows_keyboard_workaround` feature is enabled (focuses the
  plugin window whenever egui wants keyboard input; BillyDM/egui-baseview#20). Belt and braces: "Paste" buttons next
  to every text field (arboard clipboard), since DAWs do not always pass the keyboard on.

Not verified yet after these changes (need an FL restart to load the new plugin + script): typing in the panel,
channel lanes and instrument knobs, Save version -> Version + Notes, Device stamps in FL, Go back re-placing knobs.

## A10. Vocabulary: branch; the panel overhaul

- **line -> branch, idea -> branch, project-wide** (Tabor: "line" collided with "lane"). Code: `BranchId`,
  `BranchState`, `main_branch()`, `Chains::branches/branch_stamps/branch_name/branch_base`, `Payload::Branch`
  (`BranchPayload { name, from }`), `Kind::Branch`, `Song::new_branch`, IPC `Request::NewBranch/SwitchBranch/
  Branches`, `BranchInfo { branch, name, current, parent, from, from_pos }`. Stamps on disk keep reading the old
  JSON field name (`#[serde(alias = "line")]` on `StampBody.branch`); hashes are unchanged (postcard has no names).
  HANDOFF Part B and plan.md's older sections still say "line" in places; the Words section of plan.md was updated.
- **Panel overhaul** (`crates/flit-plugin/src/ui.rs`): palette taken from `flit_logo.png` (charcoal base, lavender
  text, burgundy accent, rust warnings, green link), Segoe UI when present, a left navigation rail (Home, Crew,
  Timeline, Chat, Settings), cards with a title and a one-line explanation each, Paste next to every field, status
  pills (helper / DAW link) in the top bar, the logo in the top bar and on the home screen
  (`crates/flit-plugin/assets/flit_logo_{256,96,48}.png`, made from the big logo; the 128 px one Tabor added is not
  used). Editor is 960x640 now. Timeline = Branches card (tree: Main at the root, each branch under the one it
  forked from, click to switch, "New branch from this moment" from the scrubbed row), Lanes card (Bring in),
  Moments card (lenses, scrubber with Exact view, rows with Go back / New branch / Undo / Pin, "who did this").
- Listen scrub is still not built.

## A11. Tabor's second round of reports (same day)

- **Undo/Go back of a plugin add did nothing in FL.** FL's API cannot delete or load plugins; the nearest thing is
  the mixer slot's enable switch (`REC_Plug_Mute` + `mixer.getTrackPluginId`, via `processRECEvent`; script command
  `set_slot_enabled`, VERIFY the value scale). `FlBridge::set_device` now switches a slot off when the moment has
  it empty and on again when the plugin is back, with a notice ("switched off; remove it yourself if you want it
  gone"); a slot holding a different plugin is switched off and the user is told what was there. Channel-rack
  generators cannot be bypassed this way (manual step). Notices travel as `HostEvent::Notice` ->
  `LiveLoop::notices` -> panel.
- **Panel too narrow / not adjustable.** nih_plug_egui's `ResizableWindow` wraps the content: drag the bottom-right
  corner and nih-plug asks FL to resize (VST3 host->plugin resize is not implemented upstream, plugin->host is).
  Default 1200x760; cards go side by side only above 900/1000 px of width.
- **Project <-> song auto-link.** The song lives in the project's folder (`.flit` beside the `.flp`), and `song.json`
  now records the project file name (`Song::project_file/set_project_file`, set on Share this project and the first
  time a song meets its project). Every second the agent compares the bridge's current project with the open song
  (`State::auto_link`): project folder holds a song -> it is opened; no song there -> `ProjectLink::Unlinked` and Home
  offers "Make <file> a song" (one click, no typing); a song from another folder is open -> `Mismatch` and **live
  edits are paused** (the agent reads the host but stamps nothing) until the user makes the project a song or closes
  the song; same folder but another file name -> `OtherFile`, also paused. Status carries `link` and `paused`; the
  top bar shows "edits paused".
- **Names.** `Payload::Profile { display_name }` (`Kind::Profile`): a fact on Main that only its signer can stamp
  (it is always about the signer), folded into `Chains::names`, not undoable, survives removal, 1..64 printable
  chars. Settings > "Your name" stamps it into every song you open (`State::announce_name`); the song status carries
  `names` (your nickname for someone wins over their own name); the panel shows names everywhere, short codes on
  hover. Test: `display_name_is_a_fact_only_its_name_can_set`.
- **Help tab**: every concept by purpose (people, the song, time, plugin modes, what is kept when, the FL link).
- **"Everything says 1.1"**: rows showed the DAW playhead position at the moment of the edit; with the transport
  stopped that is bar 1 beat 1. Rows now show when it happened (relative, exact UTC on hover) and the bar only when
  the playhead had moved; chat "jump" appears only for moments with a position.
