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
NOTE: after the update pack, the installed build is from BEFORE the pack; everything must be rebuilt + reinstalled
(P5) before Tabor tests. Old `.flit` folders (stamps/ layout) are refused by the new core: Tabor's test song must
be recreated ("Make it a song" again). The old identity.key is adopted as the first Name automatically.

## Decisions taken while applying update.md (pack wins over older text)
- Vocabulary in code: Name (root key = `Id`), Being (`BeingId`, `BeingRef`), Act, Fact, Keeper, History
  (`HistoryId`, main = all-zero hash), Track (`TrackId`, no owners), Thing (Song | SongSettings | Track | Device
  | Presence(being)), Merge (was Bring), Moment = act+fact. UI words: the table in FLIT_SPEC.md section 0
  (2026-10-11: users see Studio for Name, member for Being, moment for act and fact; org, album). Code names stay.
- Order: act_seq per being per history; keeper fact_seq per thing per history + seal_seq per history. Time
  (`stamped_at` in the act, `received_at` local sidecar) is display only. Lint test `tests/lint.rs` enforces it.
- Storage: `.flit/format` (=1), `.flit/spines/<name-hex>.tree` (acts, JSONL frames {h, p, act}),
  `.flit/spines/keeper.tree` (facts {h, p, fact}), `.flit/blobs`, `.flit/sidecar/received.jsonl` (deletable).
  Torn tail is truncated on open. Old `stamps/` / `events.jsonl` songs are refused (dev-only data), no migration.
- Keeper: default = genesis Name (+ machine from the Keeper act Song::create stamps); `Keeper` act (owner, on song
  settings) moves it; `Song::set_keeper(Some(KeeperKey))` makes this machine seal; the agent claims the role on
  open / on Keeper acts when (name, machine) match (`claim_keeper_if_mine`). Peers apply acts optimistically
  (view = sealed fold + unsealed acts) and mark them sealed when the fact lands. Decline = same property sealed
  since based_on by ANOTHER being -> gate fact into history "<Being>'s version of <property>" (also used for the
  offline-overlap case; the pack's "<Being> offline" name is not used since the keeper cannot tell offline from
  online without time). Version hold: act.app_version != required -> Held (applied live, unsealed); once that
  being stamps a later act with the required version, the old one is sealed aside into "<Being> on FLit <ver>".
  The act that sets RequiredVersion is exempt. The keeper's own acts travel once, as MOMENTs (outbox dedupe).
- MOMENTs apply in seal order: an early moment (seal_seq ahead) waits with its act (`early_facts`); an act from a
  Name/being not yet approved/declared waits (`waiting_crew`) when it comes from a peer (`Chains::import_act`);
  own acts (`Chains::append_act`) fail loudly. The agent re-announces its beings when a Crew act lands / every 1 s.
- Notes are a delta (`NotesPayload { clip, add, remove }`); property = clip + note keys, so parallel edits of
  different notes both land. flit-host computes the delta on save (`notes_delta`).
- Equivocation: second act with a used act_seq -> kept as evidence, being held by the keeper, an owner's agent
  auto-stamps `Flag { Equivocation }`, `CrewAction::Pardon` (new) releases.
- ai/scripted beings: `Song::act` routes track edits without a grant into "<Being>'s history"; peers reject ai
  acts aimed at main without a grant.
- Track ids: `fl-track-N` (0 = master, TrackKind::Master; the old "mix" id is gone), `fl-channel-N`.
- Presence: HostEvent::Presence (bridge) or Request::Presence (plugin) -> LiveLoop settles 500 ms -> Presence act
  on Thing::Presence(being). Agent pushes Push::Presence rows every 500 ms on change; tints the DAW
  (`show_presence`, setting show_presence_in_daw) with the folded presence (at the shown moment when scrubbing);
  Follow { being, keep } navigates the DAW. FL: script events "focus" (window + selected track/channel, only on
  change, ~200 ms), commands set_track_color / set_channel_color / select_track / select_channel / show_window;
  colors in snapshot; tints restored before Save (VERIFY ON WINDOWS: setTrackColor format, changed flag).
- Fold vs DAW (§20): `flit_host::diff(target, HostModel)`; `Receiver::apply_state(bridge, state, snapshot)`:
  read_state -> diff -> cost > SNAPSHOT_THRESHOLD(60) and bridge.load_snapshot -> load nearest Version file ->
  re-diff -> apply_diff. FakeHost counts device_loads/snapshot_loads (tests in bridges/fake/tests). FL: read_state
  yes, load_snapshot no.
- Protocol: SyncMsg::{Hello{heads}, Act, Moment, Want, After(heads), Bye}; `Node::publish()` drains
  `Song::take_outbox()`; `Node::catch_up()` asks peers for everything after my heads (on PeerUp / reconnect).
- Name management: `flit_core::NameStore` (<data>/names/<hex>/{secret, profile.json}, names/current); recovery
  code = base32 of the key in 13 groups; backup file = BLAKE3-stretched passphrase + BLAKE3-XOF stream + keyed-
  BLAKE3 tag. Agent: Request::{Names, SwitchName, NewName, Logout, RemoveName, BackupName, ImportName,
  RecoveryCode, ImportRecovery, Beings, ActAs, NewBeing, UpdateBeing, RevokeBeing, Grant, Flag}.
- Updates (§22): data side done; `flit_agent::updates` reads a release manifest (file path or URL via curl)
  from Settings > Updates (update_manifest_url, channel), compares versions; installing/switching binaries is
  the installer's job; GitHub Releases/CI are Tabor's.
- Fonts: IBM Plex Sans/Mono Regular+Medium fetched into crates/flit-plugin/assets/fonts (OFL); theme.rs embeds them.

## Phases and status (pack 1 = update.md sections 0-24)
- [x] P0 this file
- [x] P1 core rewrite
- [x] P2 host / agent / net / ipc / bridges / FL script
- [x] P3 plugin UI: theme.rs (tokens, Plex fonts, style gallery), worker.rs (IPC v2), ui.rs (top bar with Name +
      acting-as, rail, Timeline bar docked at the bottom with history switcher / lens / Acts-Facts-Time / tape /
      Live / Merge, status strip, Merge dialog, presence on tracks, Beings + Names screen, Settings > Updates),
      ui/tutorial.rs (first run + Help; written by Tabor's second session, deleted by mistake in my rewrite,
      recovered from that session's transcript and ported to the new backend: Name card shows the private key
      once, beings skip the key screen), ui/help.rs (concept list, second tab of Help).
      P3 gaps (listed in FLIT_SPEC.md): Listen scrub + Listen buttons, activity view, waveform overlays and
      "Keep both" in Merge, icon set, UI scale, QR pairing for delegated beings.
- [x] P4 docs: FLIT_SPEC.md (pack 1 + as-built table), HANDOFF.md Part A2 (A12.x), plan.md pointer
- [x] P5 build/test/install: 94 Rust tests + 10 script tests green; release built and installed 2026-10-11 02:40
      (agent, CLI, FL script, per-user VST3/CLAP). System VST3/CLAP copy NOT refreshed (needs UAC):
      `powershell -ExecutionPolicy Bypass -File tools\install\install-windows.ps1 -NoBuild -SystemPlugins`
      with FL closed. Nothing of the pack has run inside FL yet.

## NEXT: "4.5" = PACK 2 (update.md from the line "4.5 =" to the end). Tabor: "only worry once you are done
## with everything else". Not started. Its eight sections:
1. Organizations: own spine + keeper, countersigned membership (Invite + Accept), song access org | private,
   MAY check extended, last owner cannot leave.
2. Albums: personal (on the Name's personal spine) and org albums; ordered song lists as facts.
3. Frontend: Library rail (SONGS / ALBUMS / ORGS / INVITES), org page, new-song dialog (under Me or org),
   song settings "Org & access", album page, invites inbox, badges, quiet empty states.
4. Storage: orgs/<org>/spines/..., personal spine under the Name's profile.
5. Tests: eight cases (invite without accept, access=org vs private, removed member, last owner, personal album
   privacy, org album locked rows, album facts on a timeline, access change in History).
6. The Shelf (later, not first build): listings, self-hosted shelves, open songs with guest histories.
7. Standalone app + thin plugin: flit-agent becomes the FLit app with its own window; plugin = compact panel.
8. The move, step by step: 8.1 crates/flit-ui shared widgets, 8.2 flit-app headless (mostly true already:
   the agent owns everything), 8.3 app window shell (eframe), 8.4 move screens one at a time, 8.5 compact
   plugin panel, 8.6 where each thing goes, 8.7 rules (one source of truth, tests after every step).
Suggested order: 8.1 -> 8.3 -> 8.4 items 1-9 -> 8.5, then sections 1-5 (orgs, albums) built straight into the
app, then 6. "Commit per step" is Tabor's (I never touch git): stop and report after each step instead.

## Where I am right now (PACK 2 in progress, Fable)
- 8.1 DONE: `crates/flit-ui` holds theme.rs + Plex fonts + logo, `client.rs` (Shared + the IPC pump), `host.rs`
  (Host trait: Plugin | App surface, modes), `panel.rs` (+ panel/tutorial.rs, panel/help.rs). The plugin's ui.rs
  is a 100-line wrapper (PluginHost over nih-plug params); worker.rs keeps only the audio half.
- 8.3 DONE (shell = the full UI already): `crates/flit-app` = flit-agent embedded on a tokio thread + eframe
  window (ClientKind::Window, an ordinary IPC client of its own helper). Launch modes: plain, `--background`
  (window on demand), `--headless`. Second launch -> Request::OpenWindow -> window forward (native ShowWindow,
  because a hidden eframe window gets no repaints). Close = hide; Settings > Quit FLit really quits.
  Plugin auto-starts `flit-app --background` when installed, else flit-agent. IPC protocol = 3
  (ClientKind::Window, Request::OpenWindow{song,spawn}, Push::OpenWindow). `flit_agent::cli` shares args/lock/log.
- Fixes on the way: old pre-pack `.flit` beside a project is moved aside (`.flit-before-spines`) by Song::create
  and no longer counts as a song (Song::exists), auto_link says a failure once; rail notices one line + hover.
- FL crash on plugin removal / project change: FLit.clap_unloaded c0000005 = the IPC reader thread outlived
  the DLL. Fix in flit-ipc Client::drop (CancelSynchronousIo + join). Needs reinstall with FL closed.
- FL crash fix VERIFIED headless (`dropping_a_client_ends_its_reader_thread`) and installed 03:33 with FL closed
  (agent, app, per-user + system CLAP/VST3). Tabor to confirm in FL.
- PACK 2 sections 1, 2, 4, 5 BACKEND DONE (Tabor: backend first, frontend together after):
  core `org.rs` (ContainerKind Song|Org|Personal in the genesis act's `property` = "container:org"/"container:personal",
  Access Private|Org, AiPolicy, OrgDefaults, AlbumAction/AlbumState), CrewAction::{Invite, Accept, Decline, Leave,
  Demote} (+ CrewState.invited, check(kind), apply_by), SettingsChange::{Org, AiPolicy, Label, Color, Defaults,
  Archived}, Payload::Album on Thing::Album(id) (Create on the container thing; id = Create act), Kind::Album,
  Lens::Album, SongState.albums, SongSettings.{org, access, ai_policy, label, color, defaults, archived};
  verify: invited Names may Accept/Decline; songs with access = org admit `Chains.org_members` (set by the net
  layer from the held org container); orgs refuse sound, songs refuse albums; album curation = owners + creator.
  Song API: create_kind, kind/org/access/albums, invite/accept/decline/leave/demote, set_org/set_ai_policy/
  set_label/set_color/set_defaults/set_archived, create_album/album, set_org_members. Tests: `tests/orgs.rs` (6).
  net: Node holds `containers` (orgs + personal shelf) next to the open song; a sync session is per
  (peer, container) (`Hello.song` names it; initiator writes first); `broadcast`/`import_from`/`catch_up`/
  `connect_crew`/`publish` per container; `members_of` = crew + invited (orgs) + org members (songs with access
  org); JoinHandler welcomes invited Names (orgs) and org members (access = org songs) without approval;
  `invite_to`, `request_join_container`, `accept_invite_container`, `forget_invite`, `refresh_org_links`;
  NetEvent::Invite has `kind`. Test `org_invite_accept_and_song_access`.
  agent: `<data>/orgs/<hex>/` (org containers) + `<data>/names/<hex>/personal/` (shelf, made on first run),
  keeper claimed per container, pending Accepts retried, registry `song_notes` (id/org/access per song folder);
  Requests Library/NewOrg/OrgMembers/OrgInvite/AcceptOrgInvite/DeclineInvite/OrgLeave/OrgRemove/OrgPromote/
  OrgDemote/OrgSettings/OrgMoveKeeper/SetSongOrg/SetAiPolicy/NewSongIn/Albums/NewAlbum/AlbumAct/AlbumTimeline;
  rows OrgRow/OrgMemberRow/AlbumRow/AlbumSongRow/InviteRow/LibraryView (Status/SongStatus left untouched so the
  other session's frontend pass does not collide). Test `orgs_albums_and_org_access_over_ipc`.
  flit-ui client.rs: Shared.{library, org_members, albums, album_timeline}; Library+Albums refetched after org/album
  requests. Not built: org album covers over the wire (blobs serve from any container; fetch writes to the song),
  personal shelf sync to the Name's other machines (same mechanism as orgs, not wired), the Shelf (6).
- VOCABULARY (Tabor, via the other session): UI words only: Name = Studio, being = member (Studio member),
  act/fact = moment (filters still act/fact), Organization = org, Song = song, album = album. Core names stay.
- Library screen DONE (first build): `flit-ui/src/panel/library.rs`, rail entry "Library" (Songs with "New song
  under Me | org" and the open song's "Org & access" + album chips; Albums + album page with order/add/remove/
  curate and the album's moments; Orgs + org page: Songs | Albums | Members (invite by Studio ID, promote/demote/
  remove) | Settings (name, color, keeper, defaults, leave/archive); Invites inbox with Accept/Decline). Drawn by
  the headless panel test.
- 8.5 compact plugin DONE: `flit-ui/src/compact.rs` (connection, song · history, who is here, slim strip of the
  latest moments with Live/Showing, mode + name, [Open FLit]); plugin modes are the four Tabor asked for (Track
  send/receive, Talk send/receive; the `master` param id is gone, old projects fall back to Track). The session
  link is an instance NAMED "master": Track "master" sends the reference mix + transport (any Track instance
  reports transport; the helper stamps a change once), Receiver "master" hears the reference mix in place of the
  local one (`Shared.master` flag read by the audio thread). Editor 520x230. `hear_reference` param removed.
  The helper routes the reference mix to Receiver{"master"} and Talk voice to TalkReceive clients (no Panel kind).
- IPC PROTOCOL_VERSION = 4 (Request/Response grew): plugin + app must be installed together.
- INSTALL PENDING: needs FL closed + the helper stopped (`Get-Process flit-app, flit-agent | Stop-Process`), then
  `powershell -ExecutionPolicy Bypass -File tools\install\install-windows.ps1 -NoBuild -SystemPlugins`
  (release binaries + bundle are built by the background job; check scratchpad test5.log).
- Transport from many Track instances: the helper stamps Play/Stop once (checked against the folded transport
  state; `transport_from_many_track_instances_is_stamped_once`). Tempo was already settled by value.
- Tray icon DONE (`flit-app/src/tray.rs`, tray-icon 0.26): left click / "Open FLit" show the window natively
  and ask for a frame; "Quit FLit" ends the app. Smoke-tested (hide on close, back on relaunch).
- The other session owns `bridges/fl` now (HostPort trait over the loopMIDI port; loopMIDI dev-only until M6):
  hands off that crate and its HANDOFF notes.
- Album covers DONE: `blobs::fetch_into(container)`; a SetCover act makes members fetch the PNG into the org's
  (or shelf's) blob store like a project file; `Request::AlbumCover { path }` stores + stamps, `ContainerBlob`
  reads; the album page shows the cover (path field for now; a file dialog is frontend polish).
- Tabor's product calls (2026-10-11, late): the APP is what you open; the plugin is only for audio on tracks you
  choose (Track send / Receiver / Talk), never for switching songs; a master-track instance is optional (name it
  "master" to send the reference mix). FL is controlled and heard from outside through the bridge, no VST needed.
  Done for that: `Request::OpenInDaw { dir }` (opens the song here and its project with the OS default app = FL
  for .flp; a song without a project starts FL64.exe from Program Files\Image-Line so you save into the folder;
  FLIT_DAW overrides the exe); Library song rows got "Open in FL Studio", New song got "Create and open FL Studio";
  the compact panel shows "<project> is not a song yet [Make it a song]" (ShareProject) and the mismatch warning.
  Timeline bar: the live edge rule is explicit now (`PanelState.anchor`): scroll 0 and nothing selected = the
  newest moment at the right, new ones land there; scrolled back or showing a past moment = the view holds on
  the anchored moment while new ones arrive, until Live or a scroll back to the edge.
- VERIFY IN FL: "Open in FL Studio" with FL already running (FL asks about the current project), the .flp file
  association, the compact panel's Make it a song, typing in the compact panel.
- NEXT: frontend polish with Tabor (he asked: backend first, then frontend together); personal shelf sync to a
  Studio's other machines (needs a per-machine key story: two machines with
  one Name key are one iroh endpoint id); the Shelf (pack 2 §6, later).

## 2026-10-11 crash on opening the panel in FL (fixed, reinstalled incl. system VST3/CLAP)
Windows event log: FL64 fault in FLit.clap, c0000409 (a Rust panic aborts the host). Cause: the panel used the
Plex font families on its first frame, before egui had loaded them ("FontFamily::Name is not bound to any fonts").
Fix: theme installed in the editor's build step and the first frame of every context draws nothing
(`ui.rs::draw`). New test `panel_draws_in_every_first_run_state` draws the whole panel headless in every state;
run it before any install. Crash evidence lives in: Event Viewer > Application (ids 1000/1001).
