# FL Studio remote-control research report (Flapi, FL API stubs, FL MCP projects, PyFLP, FL scripting facts)

All repos were shallow-cloned into `SCRATCH = /tmp/claude-1000/-home-tabor-github-flit/e8778825-7e2d-43d3-a4f8-408f0a424874/scratchpad/flresearch/` (`Flapi/`, `FL-Studio-API-Stubs/`, `flstudio-mcp/`, `PyFLP/`, plus `flapi_pip/` = unpacked PyPI wheel flapi 1.0.1, `readmes/` = READMEs of other MCP projects, `manual/` = FL manual pages as text, `scripts/` = the Python scripts I ran). Only files written into the project: `/home/tabor/github/flit/bridges/fl/flp/samples/FL 20.8.4.flp` and `/home/tabor/github/flit/bridges/fl/flp/samples/multi-channel.flp`.

Research date: 2026-10-09. Venv: `fl-studio-api-stubs 37.0.1`, `pyflp 2.2.1`, Python 3.12.

---

## 0. Executive summary (what matters for a Rust/Python FL bridge)

1. **The only "always-on" channel into FL Studio for third-party code is a MIDI Controller Script** (`device_*.py`) talking **SysEx over a virtual MIDI loopback (loopMIDI on Windows)**. Confirmed independently by Flapi, flstudio-mcp (FL 25.2.5), Boyan253/fl-studio-2025-ai-bridge (FL 25.2.5), karl-andres (JSON files + MIDI trigger; file I/O however is blocked on newer FL), postfader, dawhub. The script sandbox has **no sockets, no threads, and (on FL 24/25) no file writes** (`open(...,"w")` -> `SystemError`), see §3.2.
2. **Wire format consensus**: SysEx `F0 7D <magic> ... F7`, payload **base64 (7-bit clean)**, request-id correlation, FL -> client response via `device.midiOutSysex(bytes)`, **FL routes the output by matching the input/output "Port" number in MIDI Settings**. Per-message payload must stay **< ~1000 bytes** on Windows (Flapi splits at 1000; flstudio-mcp measured 1000 B OK / 2000 B silently dropped). Chunk long messages (Flapi Protocol v2 has a continuation byte design).
3. **Reading/writing third‑party VST params works** via `plugins.getParamCount/getParamName/getParamValue/setParamValue/getParamValueString(paramIndex, index, slotIndex)`; VST wrappers report **4240** params (4096 real + 128 CC + 16 aftertouch) with real names at low indices (FabFilter Pro‑C 3: 100 named; Serum 2: 541+ named). Mixer-track effect = `(index=track, slotIndex=0..9)`, channel generator = `(index=channel, slotIndex=-1)`.
4. **Detecting user parameter changes**: no per-parameter event. Use `OnRefresh(flags)` with `midi.HW_Dirty_ControlValues (4096)` / `HW_Dirty_RemoteLinkValues (512)` / `HW_Dirty_Mixer_Controls (4)` as a *hint* then re-poll, or poll in `OnIdle()` (~20 ms, keep it tiny). `OnDirtyMixerTrack(index)` / `OnDirtyChannel(index)` tell you *which* track/channel. **VERIFY** which flags fire for plugin knob tweaks.
5. **Project path is NOT exposed** by the scripting API. Closest: `ui.getProgTitle()` (FL window title, usually contains the project filename), `general.getProjectTitle()/getProjectAuthor()/getProjectGenre()` (API v38 – song-info title, not file name; not in stubs 37.0.1), `general.getChangedFlag()`, `OnProjectLoad(status)`. Fall back to parsing the window title or reading FL's recent-files registry **(VERIFY)**.
6. **PyFLP 2.2.1 (GPL‑3.0) is abandoned (last commit 2023‑07) and crashes on Python 3.12** (`TypeError: <enum 'EventEnum'> has no members`, issue #183). A 10‑line monkeypatch fixes parsing (§5.3). Round-trip save of FL 20.8.4 files is byte-identical. **FL 21/2024/2025 files**: parse "works" for channels/patterns/notes/mixer, but the **playlist item struct changed (issue #200, #199, #177)** so arrangement/playlist data is missing/warned; unicode errors reported (#203). Treat PyFLP as "FL 20 era, read-mostly".

---

## 1. Flapi (MIT) — remote REPL over MIDI SysEx

Source: `SCRATCH/Flapi` (GitHub main, commit dc7794b, 2026‑07‑02, **unmaintained, mid-refactor**) and `SCRATCH/flapi_pip/flapi` (PyPI `flapi==1.0.1`, the last working release). License: `Flapi/LICENSE.md` = **MIT**, "Copyright (c) 2023 Maddy Guthridge, dimentorium". Docs: `Flapi/README.md`, `Flapi/Protocol.md` (protocol v2.0), `Flapi/Unmaintained.md`.

### 1.1 Architecture
- Client (`pip install flapi`, deps `mido[ports-rtmidi]`, `fl-studio-api-stubs>=37`) monkey-patches every function in the stub modules `playlist, channels, mixer, patterns, arrangement, ui, transport, plugins, general` (`FL_MODULES` in `_consts.py`) so that calling `transport.start()` locally sends `eval("transport.start()")` to FL and returns the pickled result (`__decorate.py`: `fl_eval(f"{module}.{func_name}({params})")`).
- FL side = **two** device scripts installed to `~/Documents/Image-Line/FL Studio/Settings/Hardware/Flapi Server/` (`cli/util.py::output_dir`): `device_flapi_receive.py` (`# name=Flapi Request`) and `device_flapi_respond.py` (`# name=Flapi Response`, `# receiveFrom=Flapi Request`). The receive script `exec`/`eval`s code and forwards the reply to the respond script via `device.dispatch(i, 0xF0, msg)`; the respond script re-emits it with `device.midiOutSysex(...)`. Two scripts were used because a script can only `midiOutSysex` to the output port linked to *its own* input port; this is the same single-port-number trick flstudio-mcp uses with one script (§3), so **one script with matching in/out port numbers is enough**.

### 1.2 MIDI port setup (Windows)
README steps: `pip install flapi` → `flapi install` (copies server dir) → **loopMIDI: create two virtual ports named exactly `Flapi Request` and `Flapi Response`** → restart FL → in *Options > MIDI Settings* give each port a unique port number in **Outputs**, set the **Input** port numbers to match, assign "Flapi Request" port to the "Flapi Request" script and "Flapi Response" port to the "Flapi Response" script. On macOS the client creates virtual ports itself (`mido.open_output(name, virtual=True)`); **this is impossible on Windows with rtmidi** (`__enable.py` catches `NotImplementedError` → `FlapiPortError`), hence loopMIDI. Port matching in `open_port()` is case-insensitive and tolerates the trailing number Windows appends ("Flapi Request 1").

Known user reports (GitHub issues): #32 — loopMIDI gave no response for one user, LoopBe30 worked (likely misconfig; also notes PyPI 1.0.1 server constants `EXEC=0x04/EVAL=0x05` differ from repo main `REGISTER_MESSAGE_TYPE=0x04/EXEC=0x05`, i.e. **PyPI client and GitHub-main server are incompatible**). #37 — macOS virtual ports not created. #11 — `ModuleNotFoundError: No module named 'logging'` inside FL (older FL builds lack some stdlib modules; `logging` is used by `device_flapi_receive.py`).

### 1.3 Wire format (1.0.1 as shipped; `server/consts.py`, `__comms.py`, `server/flapi_response.py`)
```
F0 | 7D 46 6C 61 70 69 ('Flapi' after 0x7D non-commercial ID) | origin | client_id | msg_type | [status (responses only)] | data... | F7
origin: 0x00 client, 0x01 server, 0x02 internal (receive->respond dispatch)
client_id: random 0x01..0x7F chosen by client; 0x00 = broadcast (server goodbye)
msg_type: 0 CLIENT_HELLO, 1 CLIENT_GOODBYE, 2 SERVER_GOODBYE, 3 VERSION_QUERY, 4 EXEC, 5 EVAL, 6 STDOUT
status:   0 OK, 1 ERR (data = base64(pickle(exception))), 2 FAIL (data = base64(utf8 text))
data:     EXEC/EVAL request = base64(utf8 source); EVAL response = base64(pickle(result)); STDOUT = base64(text); VERSION = 3 raw bytes
```
Encoding: **base64 of UTF‑8 / pickle** — every byte is ASCII < 0x80, so it is 7‑bit clean with no bit-packing. Client busy-waits for the reply (`TIMEOUT_DURATION = 0.1 s`, `receive_message()` loops `res_port.receive(block=False)`); `CONNECTION_TIMEOUT = 60 s` in the CLI. Discovery = send `CLIENT_HELLO`; server ignores it if that client_id is already in use (client then picks another id). Server also answers FL's universal device enquiry `F0 7E 00 06 01 F7` with an identity reply (`DEVICE_ENQUIRY_RESPONSE`). Stdout inside FL is captured (`server/capout.py` swaps `sys.stdout` for a StringIO subclass; note FL exposes `_io` not `io` for `StringIO/_TextIOBase`) and streamed back as STDOUT messages.

Protocol **v2 (Protocol.md, only partly implemented on main)** adds a **continuation byte** after client_id and the register-message-type mechanism:
> "In Windows, system-exclusive MIDI messages can't have an arbitrary length. Applications must instead pre-allocate a buffer ... Most applications only support a buffer size of 1024 bytes. ... If a message is too long, it is split at 1000 bytes, and the continuation byte is set to 1."

`flapi/flapi_msg.py::FlapiMsg.to_bytes()` implements the split (`MAX_DATA_LEN = 1000`, `itertools.batched`), `client/comms.py::try_receive_message()` reassembles. `Unmaintained.md` says oversize SysEx "silently fail to send, and can even crash FL Studio due to buffer overflow" (author believes crash fixed in current FL).

Key snippets (FL side, 1.0.1):
```python
# server/device_flapi_receive.py
def OnSysEx(event):                       # FlMidiMsg; event.sysex includes F0 .. F7
    header = event.sysex[1:len(SYSEX_HEADER)+1]
    sysex_data = event.sysex[len(SYSEX_HEADER)+1:-1]
    if header != SYSEX_HEADER: return
    message_origin, client_id, message_type, data = sysex_data[0], sysex_data[1], MessageType(sysex_data[2]), sysex_data[3:]
    if message_origin != MessageOrigin.CLIENT: return      # avoid feedback
    ...
    with capout(client_id): handler(res, data)             # exec()/eval() in per-client globals()
    res.send()                                             # -> device.dispatch(i, 0xF0, msg) to respond script

# server/flapi_response.py
def send_sysex(msg: bytes):
    if device.dispatchReceiverCount() == 0: print("ERROR: No response device found", file=sys.stderr)
    for i in range(device.dispatchReceiverCount()): device.dispatch(i, 0xF0, msg)

# server/device_flapi_respond.py
def OnSysEx(event):
    if sysex_data[0] != MessageOrigin.INTERNAL: return
    device.midiOutSysex(bytes([0xF0]) + SYSEX_HEADER + bytes([MessageOrigin.SERVER]) + sysex_data[1:])  # must include F0..F7
def OnDeInit():  # broadcast server goodbye
    device.midiOutSysex(bytes([0xF0]) + SYSEX_HEADER + bytes([MessageOrigin.SERVER, 0x00, MessageType.SERVER_GOODBYE, 0xF7]))
```
Client send (mido adds F0/F7): `req_port.send(mido.Message("sysex", data=SYSEX_HEADER + bytes([MessageOrigin.CLIENT, client_id, MessageType.EVAL]) + b64encode(expr.encode())))`.

### 1.4 Flapi limits / lessons
- Request/response is **synchronous and single-threaded on FL's UI thread**: every `eval` runs inside `OnSysEx`; a long handler stalls FL (flstudio-mcp confirms: hundreds of API calls in one tick → stall/timeout, see §3.4).
- Pickle over the wire = arbitrary code execution risk for anything that isn't your own client; fine for a private bridge.
- `OnIdle` is not used by Flapi; it is purely request-driven.
- FL's embedded Python (README): `3.12.1 (tags/v3.12.1:2305ca5, Dec 7 2023) [MSC v.1937 64 bit (AMD64)]` — so stdlib modules exist, but see sandbox notes §3.2.

---

## 2. FL Studio API Stubs (LGPL‑3.0) — exact signatures & semantics

Source: `SCRATCH/FL-Studio-API-Stubs` (IL-Group, commit 9b4c7fa, 2026‑05‑13, version **37.0.1**, README says "This version of the documentation is unmaintained ... refer to the FL Studio manual or MaddyGuthridge/FL-Studio-API-Stubs"). License `LICENSE.md` = **LGPL‑3.0‑only** (stubs only; importing them at dev time is fine). Installed in venv at `.../site-packages/{device,mixer,channels,plugins,transport,general,ui,patterns,playlist,arrangement,midi,utils,screen,launchMapPages,fl_classes}`. Source tree: `src/midi_controller_scripting/<module>/__*.py`, docs `docs/midi_controller_scripting/**`, callbacks documented in `src/midi_controller_scripting/callbacks/__init__.py` (not importable in FL).

**Version annotations: YES.** Every docstring ends with `Included since API version N.` plus optional `## API Changes` bullets (e.g. "v26: add useGlobalIndex flag"). There is **no API‑version→FL‑version table** anywhere in stubs or manual. Empirical anchors: FL 20.7 = API v1 (first release; karl-andres requires 20.7+), stubs tutorial screenshot shows "FL Studio Midi scripting version: 35", flstudio-mcp logs "FL 24+ MIDI scripting v40" and "FL 25.2.5 [build 5319], MIDI scripting v40", the current online manual lists functions up to **v45** (FL 2026). `general.getVersion()` returns the API version at runtime — log it on `OnInit`.

Module sizes (public names): device 34, mixer 69, channels 48, plugins 13, transport 20, general 24, ui 71, patterns 25, playlist 41, arrangement 9, midi 487, utils 28, screen 28, launchMapPages 12, fl_classes 9.

### 2.1 Requested functions (signature → semantics → since)
**device**
- `device.midiOutSysex(message: bytes) -> None` — "Send a sysex message to the (linked) output device. The sysex data **must include the sysex start 0xF0 and sysex end 0xF7 bytes, or FL Studio will ignore the function call entirely**." v1. (Manual: "string message".)
- `device.isAssigned() -> bool` — True if an output interface is linked to the script (so `midiOut*` works). v1.
- `device.getName() -> str` — device name. v7.
- `device.getPortNumber() -> int` — port number of the input device the script is attached to (−1 if unassigned); "If the device requires two-way communication, the output port ... should be set to the value of the input port". v1.
- `device.dispatch(ctrlIndex: int, message: int, sysex: Optional[bytes] = None)`, `device.dispatchReceiverCount() -> int`, `device.dispatchGetReceiverPortNumber(ctrlIndex)` — script-to-script messaging; receiver must declare `# receiveFrom=<sender name>`. v1/v5.
- `device.setHasMeters() -> None` — call in `OnInit` to receive `OnUpdateMeters()`. v1.
- Linked/REC helpers (`device/__fl.py`): `findEventID(controlId, flags=0) -> int` (REC_InvalidID if unlinked), `getLinkedValue(eventID) -> float` (0.0–1.0 or −1), `getLinkedValueString(eventID) -> str` (e.g. `'-5.2 dB'`), `getLinkedParamName(eventID) -> str` (v10, e.g. `'Channel volume'`), `getLinkedInfo(eventID) -> int`, `getLinkedChannel(eventId)` (v27), `linkToLastTweaked(...)` (v21). Example from docstring: `device.getLinkedValue(channels.getRecEventId(0) + midi.REC_Chan_Vol) -> 0.78125`.

**plugins** (all: `index` = channel index or mixer track; `slotIndex` = mixer slot 0–9, **−1 = channel-rack generator**; `useGlobalIndex` added v26)
- `plugins.isValid(index, slotIndex=-1, useGlobalIndex=False) -> bool` — "Audio samples are not considered to be plugins". v8.
- `plugins.getPluginName(index, slotIndex=-1, userName=False, useGlobalIndex=False) -> str` — original plugin name, or user's name if `userName=True` (v12). v8.
- `plugins.getParamCount(index, slotIndex=-1, useGlobalIndex=False) -> int` — "**VST plugins are listed as having 4240 parameters** ... first 4096 are for parameters, next 128 MIDI CC sends 0–127, final 16 after-touch per MIDI channel". v8.
- `plugins.getParamName(paramIndex, index, slotIndex=-1, useGlobalIndex=False) -> str`. v8.
- `plugins.getParamValue(paramIndex, index, slotIndex=-1, useGlobalIndex=False) -> float` — normalized 0.0–1.0. v8.
- `plugins.setParamValue(value: float, paramIndex, index, slotIndex=-1, pickupMode=0, useGlobalIndex=False) -> None` — value 0.0–1.0; pickupMode 0 none / 1 always / 2 follow FL setting (v17). v8.
- `plugins.getParamValueString(paramIndex, index, slotIndex=-1, pickupMode=0, useGlobalIndex=False) -> str` — display string ("only supported by some plugins"; in practice works for native and the probed VSTs). v8.
- `plugins.getPresetCount(index, slotIndex=-1, useGlobalIndex=False) -> int` v15; `nextPreset/prevPreset(index, slotIndex=-1, ...)` v10; `plugins.getName(index, slotIndex=-1, flag=0, paramIndex=0, ...)` v13 with `flag` FPN_Param 0, FPN_ParamValue 1, FPN_Semitone 2, FPN_Patch 3, FPN_Preset 6 (preset name at paramIndex) ...; `getColor` v12; `getPadInfo` v19. **No `getPresetName`** — use `getName(..., flag=6, paramIndex=n)`.

**mixer**
- `mixer.trackCount() -> int` — includes Master (0) and the "current track" utility track; valid 0..trackCount()-1. v1. (`mixer.getTrackCount()` v38 excludes them — not in stubs.) `mixer.getTrackInfo(mode)` → TN_Master 0 / TN_FirstIns 1 / TN_LastIns 2 / TN_Sel 3.
- `mixer.getTrackName(index) -> str` v1; `setTrackName(index, name)` ("" resets).
- `mixer.getTrackVolume(index, mode=0) -> float` — 0.0–1.0, **default/unity = 0.8**; `mode=1` returns dB (v14). `mixer.setTrackVolume(index, volume, pickupMode=0)`. v1.
- `mixer.getTrackPan(index) -> float` −1.0..1.0; `setTrackPan(index, pan, pickupMode=0)`. v1.
- `mixer.isTrackMuted(index) -> bool` v2; `mixer.muteTrack(index, value=-1)` toggle, `value` 1 mute / 0 unmute (v30 for value). **flstudio-mcp found `muteTrack(t, 1)` does NOT mute on FL 25.2.5, only bare toggle works, and FL coalesces multiple mute ops in one script tick** (`docs/PHASE1A_REPORT.md`).
- `mixer.getSongTickPos(mode=0) -> int|float` — song position in ticks (modes in midi "song tick modes"). v1. `mixer.getSongStepPos() -> int` — position in steps. v1.
- `mixer.getCurrentTempo(asInt=False) -> int|float` — v1. **Beware**: flstudio-mcp observed the *raw* value is BPM×1000 on FL 25 (`raw/1000`); stubs don't say that. Setting tempo: `general.processRECEvent(midi.REC_Tempo, int(bpm*1000), midi.REC_UpdateValue | midi.REC_UpdateControl)` (do **not** add `REC_FromMIDI`), or `mixer.setCurrentTempo(tempo, asInt=0)` v38 (manual only).
- `mixer.getTrackPluginId(index, plugIndex) -> int` — "Plugin IDs are represented as `((track << 6) + index) << 16`" (manual's `(track << 6 + index) << 16` has wrong precedence); used as the REC-event base for mixer track/plugin params and returned by `ui.getFocusedFormID()` for a focused effect. v1. `mixer.isTrackPluginValid(index, plugIndex) -> bool` v1.
- `mixer.getEventValue(index, value=MaxInt, smoothTarget=1) -> int`, `mixer.getEventIDName(index, shortname=0) -> str`, `mixer.getEventIDValueString(index, value) -> str`, `getAutoSmoothEventValue`, `remoteFindEventValue` — all documented as "HELP WANTED / ???" in stubs (v1). `index` is a REC event id; `getEventIDName` returns its name. **VERIFY**; prefer `device.getLinkedValue*` which are documented.
- `mixer.getLastPeakVol(section) -> float` — section 0 L / 1 R; 0.0 silence, 1.0 = 0 dB, >1 clipping. v9. `mixer.getTrackPeaks(index, mode) -> float` mode 0 L / 1 R / 2 max. v1 (meaningful only while playing; flstudio-mcp uses it for a running peak watch).
- Routing: `setRouteTo(index, destIndex, value: bool, updateUI=False)` + `afterRoutingChanged()` v1; `getRouteSendActive(index, destIndex)`; `getRouteToLevel/setRouteToLevel` v36 (manual only, not in stubs); `getPluginMixLevel/setPluginMixLevel(index, plugIndex[, value])` v37 (manual only). `getTrackColor/setTrackColor`, `isTrackSolo/soloTrack`, `armTrack/isTrackArmed`, `getTrackStereoSep` v12, `getTrackDockSide` v13, `isTrackSlotsEnabled` v19, `getSlotColor` v32, `getActiveEffectIndex() -> (track, slot)|None` v25, `getTrackRecordingFileName(index) -> str` v1 ("Files can't be opened in FL Studio's Python interpreter due to disk access being disabled for security reasons").

**transport**
- `transport.isPlaying() -> bool` v1; `transport.start()` = **play/pause toggle**; `transport.stop()`; `transport.record()` toggle; `isRecording()`; `getLoopMode() -> 0 pattern / 1 song`; `setLoopMode()` toggles.
- `transport.getSongPos(mode=-1) -> float|int` — default fractional 0..1 of song; `SONGLENGTH_MS 0`, `_S 1`, `_ABSTICKS 2`, `_BARS 3`, `_STEPS 4`, `_TICKS 5` (midi constants). Position is song or pattern depending on loop mode. v1 (mode v3).
- `transport.setSongPos(position, mode=-1)` — same modes 0–2 (fractional needs float). v1 (mode v4).
- `transport.getSongLength(mode) -> int` — full song only; use `patterns.getPatternLength()` for pattern mode. v3.
- `transport.globalTransport(command, value, pmeflags=2, flags=15) -> int` — generic FPT_* commands (`midi.FPT_Play 10, FPT_Stop 11, FPT_Record 12, FPT_Undo 20, FPT_Cut 50, FPT_Copy 51, FPT_Paste 52, FPT_Save 92, FPT_SaveNew 93`); Boyan253 uses `FPT_Copy/FPT_Paste` to place playlist clips (there is **no placeClip API**).

**general**
- `general.getRecPPQ() -> int` — project timebase (ticks/quarter, default 96). v8. `general.getRecPPB() -> int` — PPQ × beats per bar (ignores time-sig markers). v1.
- `general.getVersion() -> int` — **MIDI scripting API version**, not FL version (use `ui.getVersion(mode)`: 0 major,1 minor,2 release,3 build (ints), 4 `"Producer Edition v20.8.4 [build 2553]"`, 5 full, 6 `"Windows - 64Bit"`). v1/v7.
- `general.getChangedFlag() -> int` — 0 unchanged, 1 changed since save, 2 changed but unchanged since autosave. v1.
- `general.saveUndo(undoName: str, flags: int, update: bool = True) -> None` — flags `UF_None 0, UF_EE 1, UF_PR 2, UF_PL 4, UF_KNOB 32, UF_AudioRec 256, UF_AutoClip 512, UF_PRMarker 1024, UF_PLMarker 2048, UF_Plugin 4096, UF_SSLooping 8192, UF_Reset 65536`. v1. Also `undo()`, `undoUp()`, `undoDown()`, `getUndoHistoryPos/Count`, `setUndoHistoryPos`, `getUndoLevelHint`.
- `general.processRECEvent(eventId: int, value: int, flags: int) -> int` — v7. Stubs warn: "incomplete, poorly documented, and filled with hidden bugs ... providing an invalid value can lead to very strange behavior and sometimes crashes". With `REC_MIDIController`/`REC_FromMIDI` the value is 0..`midi.FromMIDI_Max (1073741824 = 2^30)` and FL rescales; without it the value is in the parameter's native units (tempo = BPM×1000).
- `general.safeToEdit() -> bool` v29 ("safe to use setter functions"). `general.getProjectTitle()/getProjectAuthor()/getProjectGenre() -> str` **v38, manual only**. `dumpScoreLog(time, silent=False)` v15.

**channels**
- `channels.channelCount(globalCount=False) -> int` v1 (groups respected unless global). `channels.getChannelName(index, useGlobalIndex=False) -> str` v1 (useGlobalIndex v33). `getChannelVolume(index, mode=False, useGlobalIndex=False) -> float` 0–1 (default `midi.ChannelDefaultVolume = 1000/1280 = 0.78125`; `mode=True` → dB). `getChannelPan(index, useGlobalIndex=False) -> float` −1..1. `setChannelVolume(index, volume, pickupMode=0, useGlobalIndex=False)`. `getTargetFxTrack(index)`, `setTargetFxTrack(channelIndex, mixerIndex)`, `getChannelType(index)` v19 (`CT_Sampler 0, CT_Hybrid 1, CT_GenPlug 2, CT_Layer 3, CT_AudioClip 4, CT_AutoClip 5`), `getRecEventId(index) -> int` (REC base for `midi.REC_Chan_*`), `selectedChannel(canBeNone=False, offset=0, indexGlobal=False)` v5, `selectOneChannel(index)` v8, `midiNoteOn(indexGlobal, note, velocity, channel=-1)`, `getGridBit/setGridBit(index, position[, value])`, `getChannelColor/setChannelColor`, `incEventValue(eventId, step, res=1/24)`. **No `channels.getChannelPluginId`** — channel generators use `plugins.*(index=channel, slotIndex=-1)` and REC ids via `getRecEventId(ch) + midi.REC_Chan_Plugin_First (32768) + paramIndex` (VERIFY offset semantics).

**ui**
- `ui.getFocused(index) -> bool` — `widMixer 0, widChannelRack 1, widPlaylist 2, widPianoRoll 3, widBrowser 4, widPlugin 5 (getFocused only), widPluginEffect 6, widPluginGenerator 7`. v1. `ui.getVisible(index) -> bool` (0–4). `ui.showWindow(index)`/`hideWindow(index)` v1/v5.
- `ui.getFocusedFormCaption() -> str` — caption of focused FL window ("isn't necessarily the plugin's name"). v1. `ui.getFocusedPluginName() -> str` v5. `ui.getFocusedFormID() -> int` v13 — channel index for generator windows, `track*4194304 + slot*65536` (= `getTrackPluginId`) for effects, window id constants for main windows, −1 otherwise.
- `ui.getProgTitle() -> str` — "Returns the title of the FL Studio window." v1 — the only API hint at the open project's file name. `ui.setHintMsg(msg)`, `ui.isInPopupMenu()`, `ui.getHintValue(value, max)`.

**patterns / playlist / arrangement (selected)**: `patterns.patternCount()` (patterns modified from default; may be non-contiguous), `patternNumber()` (active), `getPatternName(index)`, `getPatternLength(index)` (beats), `jumpToPattern(index)` (creates/selects), `clonePattern(index=None)` v25, `setPatternName`, `findFirstNextEmptyPat(flags, x=-1, y=-1)`, `isPatternDefault` v23; `playlist.trackCount()`, `playlist.getTrackName(index)` (**playlist track indexes start at 1**), `playlist.isTrackMuted/muteTrack`, live-performance functions; `arrangement.currentTime(snap) -> ticks`, `currentTimeHint(mode, time, ...) -> "Bar:Step:Tick"`, `selectionStart()/selectionEnd()`, `getMarkerName(index)`, `addAutoTimeMarker(time, name)`, `jumpToMarker(delta, select)`. **There is no API to place pattern/audio clips in the playlist, create a new pattern from nothing other than `jumpToPattern(n)`, load a plugin, load audio, or render** (ROADMAP/docs of flstudio-mcp; `docs/ARRANGEMENT_FINDING.md`).

### 2.2 Enumerating mixer slots and parameters (pattern)
```python
import mixer, plugins
for track in range(mixer.trackCount()):            # 0 = Master
    for slot in range(10):                          # mixer effect slots 0..9 (mixer.max_slots in PyFLP = 10)
        if not plugins.isValid(track, slot):        # or mixer.isTrackPluginValid(track, slot)
            continue
        name = plugins.getPluginName(track, slot)   # userName=True for the user's rename
        n = plugins.getParamCount(track, slot)      # 4240 for VST wrappers; small real counts for native (EQ2=36, Reeverb2=15, Delay3=26)
        for p in range(n):                          # paginate! never do 4240*3 calls in one OnSysEx tick
            pname = plugins.getParamName(p, track, slot)
            if not pname: continue                  # unused VST slots have empty names
            val = plugins.getParamValue(p, track, slot)          # 0.0..1.0
            s = plugins.getParamValueString(p, track, slot)      # e.g. "-16.00 dB", "3.50:1"
# channel-rack generator: plugins.getParamName(p, channel_index, -1)
# write: plugins.setParamValue(0.42, p, track, slot)  -> read back getParamValue (FL clamps)
```
Empirical (flstudio-mcp `docs/VST_PROBE_FINDING.md`, `docs/SERUM_PROBE_FINDING.md`, FL 25.2.5): FabFilter Pro‑C 3 → 4240 count, 100 named params at idx 0–99, value strings `-16.00 dB`, `3.50:1`, `0.725 ms`; Serum 2 (generator, slot −1) → 541 named in 0–1024, strings like `"50% [-9.0 dB]"`; sampler channels return `isValid(c,-1)=False`. Preset navigation on VSTs is the generic 128-program wrapper (`getPresetCount()=128`, names "Prog 1") — **Serum's preset browser is not reachable**. Controller truncates names to 30 chars in their protocol (their choice, not FL).

### 2.3 REC event id scheme (`midi/__rec_events/*.py`, manual "REC events" table)
- Layout: each item (channel / mixer track / plugin) owns `REC_ItemRange = 0x10000` ids. Channels: `REC_Chan_First = 0 ... REC_Chan_Last = 0xFFFFFFF`; base = `channels.getRecEventId(ch)`; offsets `REC_Chan_Vol 0, REC_Chan_Pan 1, REC_Chan_FCut 2, REC_Chan_FRes 3, REC_Chan_Pitch 4, REC_Chan_FType 5, REC_Chan_PortaTime 6, REC_Chan_Mute 7, REC_Chan_FXTrack 8, ...`, plugin params at `REC_Chan_Plugin_First = REC_Chan_First + REC_PluginBase(0x8000)` .. `+0x8000-1`.
- Mixer/effects: `REC_Plug_First = 0x2000*0x10000 = 0x20000000 (536870912)`; per (track,slot) base = `mixer.getTrackPluginId(track, slot) = ((track<<6)+slot)<<16` **added to REC_Plug_First implicitly? NO — the manual says: "use above parameters together with mixer track plugin id getTrackPluginId, for example, to change volume for mixer track at 'index' use: `midi.REC_Mixer_Vol + mixer.getTrackPluginId(index, 0)`".** Absolute constants (computed from stub source, match manual's numbers 536879064/536879080): `REC_Plug_General_First 536878848`, `REC_Plug_Mute 536878848`, `REC_Plug_MixLevel 536878849`, `REC_Mixer_Send_First 536878912` (+dest track), `REC_Mixer_Vol 536879040 (0x20001FC0)`, `REC_Mixer_Pan 536879041`, `REC_Mixer_SS 536879042`, `REC_Mixer_EQ_Gain 536879056 (+band 0..7)`, `REC_Mixer_EQ_Freq 536879064`, `REC_Mixer_EQ_Q 536879072`, `REC_Mixer_EQ_Type 536879080`, **effect plugin params `REC_Plug_Plugin_First 536903680 (0x20008000) + paramIndex`** (`REC_Plug_Plugin_Last 536936447`). So a 3rd-party effect param p on (track t, slot s) ≈ `mixer.getTrackPluginId(t, s) + midi.REC_Plug_Plugin_First + p` — **VERIFY ON WINDOWS** against `plugins.getParamValue`; prefer the `plugins.*` API for params and REC only for things `plugins` lacks (mixer vol/pan/sends/EQ, tempo, master).
- Globals: `REC_Global_First 0x40000000`, `REC_MainVol +0`, `REC_MainShuffle +1`, `REC_MainPitch +2`, `REC_Tempo = 1073741829` (value = BPM×1000).
- Flags (`process_flags.py`): `REC_UpdateValue 1, REC_GetValue 2, REC_ShowHint 4, REC_UpdatePlugLabel 8, REC_UpdateControl 16 ("Without this, any changes won't be shown in FL Studio's UI"), REC_FromMIDI 32 (value is 0..FromMIDI_Max, FL rescales), REC_Store 64, REC_SetChanged 128, REC_SetTouched 256, REC_Init 512, REC_NoLink 1024, REC_InternalCtrl 2048, REC_Smoothed 8192, REC_NoLastTweaked 16384, REC_NoSaveUndo 32768`; combos `REC_Control`, `REC_MIDIController = REC_Control | REC_FromMIDI`, `REC_SetAll`. Reading via REC: `general.processRECEvent(id, 0, midi.REC_GetValue)` returns the value (**VERIFY**; documented return is "Unknown"), or use `device.getLinkedValue(id)`.
- **Packaging bug in stubs 37.0.1**: `midi.REC_Mixer_*` / `REC_Plug_*` are defined in `midi/__rec_events/mixer_properties.py` but **not re-exported from `midi/__init__.py`** (both PyPI and GitHub). In FL they exist (manual lists them). For local type-checking import `from midi.__rec_events import REC_Mixer_Vol` or hardcode the numbers above.

### 2.4 Detecting parameter changes made by the user
- Callbacks (`callbacks/__init__.py`, manual "Script events"): `OnRefresh(flags: int)` — bitwise `midi.HW_Dirty_Mixer_Sel 1, HW_Dirty_Mixer_Display 2, HW_Dirty_Mixer_Controls 4, HW_Dirty_RemoteLinks 16, HW_Dirty_FocusedWindow 32, HW_Dirty_Performance 64, HW_Dirty_LEDs 256, HW_Dirty_RemoteLinkValues 512 ("Value of remote (linked) control has changed"), HW_Dirty_Patterns 1024, HW_Dirty_Tracks 2048, HW_Dirty_ControlValues 4096 ("Changes to plugin control values"), HW_Dirty_Colors 8192, HW_Dirty_Names 16384, HW_Dirty_ChannelRackGroup 32768, HW_ChannelEvent 65536`. `OnDirtyMixerTrack(index)` (−1 = all) and `OnDirtyChannel(index[, flag])` v16 — "collect information about dirty tracks here, but do not handle the refresh — wait for OnRefresh with HW_Dirty_Mixer_Controls". `OnDoFullRefresh()`, `OnProjectLoad(status)` v16 (`PL_Start 0, PL_LoadOk 100, PL_LoadError 101`), `OnUpdateMeters()` (needs `device.setHasMeters()`), `OnUpdateBeatIndicator(value)`, `OnFirstConnect()` v17, `OnSendTempMsg(message, duration)` (hint text FL would show on the controller).
- There is **no per-parameter "value changed" callback with the event id**. Strategy: on `OnRefresh` with `HW_Dirty_ControlValues|HW_Dirty_RemoteLinkValues|HW_Dirty_Mixer_Controls` mark state dirty and emit a cheap "dirty" SysEx to the host; the host then re-reads the parameters it cares about (paginated). `HW_Dirty_RemoteLinkValues` only fires for controls that are *linked* to a MIDI control, so for arbitrary VST knobs rely on `HW_Dirty_ControlValues` **(VERIFY it fires for 3rd-party plugin knob moves)**. Also `device.linkToLastTweaked` / "last tweaked" REC ids (`REC_LastTweakedFirst = -32`) exist for "what did the user just touch" **(VERIFY)**.
- `OnIdle()` — "Called frequently (roughly once every 20ms) ... If this function runs too slowly, it can cause your script to lag" (stubs); manual: "Called from time to time". flstudio-mcp emits a heartbeat from `OnIdle` every 500 ms. **Boyan253 reports `OnIdle` is never called on FL 25.2.5 in their setup** (they use a MIDI "wake" note instead) while flstudio-mcp's heartbeat from `OnIdle` works on the same build — **VERIFY**; don't depend solely on OnIdle, keep a request-driven path.

### 2.5 Script/event object facts (`fl_classes/__init__.py`)
`FlMidiMsg` properties: `handled` (set True to stop FL processing), `status` (0xF0 for sysex), `data1/data2` (raise on sysex), `sysex: bytes` ("Contains the full event data" — Flapi slices `event.sysex[1:...]`, i.e. includes the leading 0xF0; flstudio-mcp strips F0/F7 defensively because builds differ — **VERIFY**), `timestamp` (always 0), `port`, `pmeFlags` (unsafe ops raise `TypeError("Operation unsafe at current time")`), `isIncrement`, `res`, `inEv/outEv/midiId/midiChan/midiChanEx`. `fl_classes` is **not available inside FL** — guard with `try: from fl_classes import FlMidiMsg except ImportError: ...`.

---

## 3. FL Studio MCP projects

### 3.1 rosasynthesiz/flstudio-mcp (MIT, 75★, v1.0.0 beta, FL 2025+)
Source `SCRATCH/flstudio-mcp` (commit ba94a6d, 2026‑07‑03). Key files: `fl_controller/FLStudioMCP/device_FLStudioMCP.py` (1103 lines, the FL script), `src/fl_studio_mcp/protocol.py`, `connection.py` (mido + python‑rtmidi, callback thread + `threading.Event` per request id), `daemon.py` (TCP 127.0.0.1:9787 proxy that owns the MIDI ports), `pyscript_gen.py`/`pyscript_trigger.py` (piano-roll note bridge), `docs/*.md` (excellent empirical reports on FL 25.2.5 build 5319, API v40, Python 3.12).

Setup (README + `scripts/install_windows.bat`): loopMIDI ports named exactly `FLStudioMCP RX` (server OUT → FL IN) and `FLStudioMCP TX` (FL OUT → server IN); copy script to `%USERPROFILE%\Documents\Image-Line\FL Studio\Settings\Hardware\FLStudioMCP\device_FLStudioMCP.py`; FL *Options > MIDI Settings*: **Input `FLStudioMCP RX`: enable, Controller type = FLStudioMCP, Port = 42; Output `FLStudioMCP TX`: enable, Port = 42 (same number)**; and **disable RX as output / TX as input** (otherwise `midiOutSysex` routes to the wrong port — `docs/FIX_REPORT.md` root cause 2). "View > Script output should show `[FLStudioMCP] Ready`". Full FL restart needed after installing; FL's "Reload script" was found **unreliable** for picking up edits (`docs/MIXING_ROUTING_REPORT.md` §0).

Wire format (`protocol.py`): `F0 7D 4D 43 50 <dir> <id8> <base64(json)> F7` — `7D` private manufacturer id, magic "MCP", dir `0x01 request / 0x02 response / 0x03 heartbeat`, 8‑char `[a-z0-9]` request id, base64 of compact JSON `{"v":2,"cmd":"...","params":{...}}` / `{"v":2,"ok":true,"data":...}`. Explicitly chose base64 over 7‑to‑8 packing (`docs/architecture.md`). Heartbeat every 0.5 s from `OnIdle`; stale after 3 s; command timeout 5 s.

FL-side snippet (`device_FLStudioMCP.py`):
```python
# name=FLStudioMCP
def OnInit():  _fl_version = ui.getVersion(); _send_sysex_fn = getattr(device, "midiOutSysex", None) or getattr(device, "midiOutSysEx", None); _emit_heartbeat()
def OnIdle():  # heartbeat every 0.5 s
def OnMidiMsg(event): event.handled = _handle_request_sysex(event, "OnMidiMsg")   # some builds deliver SysEx here
def OnSysEx(event):   event.handled = _handle_request_sysex(event, "OnSysEx")     # FL 21+/v40 delivers here (FIX_REPORT 3a)
def _handle_request_sysex(event, source):
    raw = bytes(event.sysex); strip leading 0xF0 / trailing 0xF7 if present; decode; dispatch; _send_message(DIR_RESPONSE, request_id, payload)
def _send_message(...): framed = bytes([0xF0]) + body + bytes([0xF7]); _send_sysex_fn(framed)
```
Measured limits (`device_FLStudioMCP.py` comments, `docs/PHASE1A_REPORT.md`): "**SysEx payloads >~1.5 KB are dropped (probe: 1000 B OK, 2000 B lost)**" → every list response is paginated by a 600‑byte JSON budget (`_paginate`), names truncated to 24 chars in lists, param pages scan ≤150 params and ≤480 B. "The FL controller-script sandbox **stalls on heavy work in a single OnSysEx tick** (hundreds of API calls, or `getGridBit` on audio channels)" → keep the controller thin, do judgement server-side. Volume: **0.8 normalized = 0 dB**: `norm = 0.8*10**(db/20)`. Tempo: `mixer.getCurrentTempo()/1000`; set via `processRECEvent(REC_Tempo, bpm*1000, REC_UpdateValue|REC_UpdateControl)` (`midi.REC_Updated` does not exist; adding `REC_FromMIDI` collapsed tempo to ~10 BPM). Mute: use bare `muteTrack(t)` toggle after reading `isTrackMuted`. Microsoft‑Store (MSIX) Claude Desktop spawns the MCP process in a context where **loopMIDI delivers no input** → hence the TCP daemon.

What works per their docs: transport, tempo, mixer/channel read+write, routing read+write (`setRouteTo`+`afterRoutingChanged`), colors, plugin param read/write for native and VST (names+strings), presets (generic only), pattern create/clone/rename, time markers, peaks while playing. What doesn't: loading plugins/audio, rendering, placing playlist clips, reading Serum presets, running a pyscript from the API (note writing needs the user to run `MCP_Apply` once per session from the Piano roll Scripting menu, then the daemon force-focuses FL and sends **Ctrl+Alt+Y "Run last script again"**; the .pyscript is regenerated with notes baked in because **pyscript file I/O (read and write) is blocked on FL 25.2.5** — `docs/NOTE_BRIDGE_SETUP.md`, `fl_pyscripts/MCP_Read_Probe.pyscript`). Pyscript folder: `~/Documents/Image-Line/FL Studio/Settings/Piano roll scripts/`. Issues: only 2 (#2 macOS support closed, #3 CI open).

### 3.2 Sandbox facts (flstudio-mcp `docs/architecture.md`, `docs/CHANGELOG.md`; Boyan253 README)
On FL 24+/25.2.5 (API v40, embedded Python 3.12.1) inside a controller script: `open(path,"w").write(..)` → `SystemError: <class '_io.FileIO'> returned NULL without setting an exception`; `os.open(..., O_WRONLY|O_CREAT)` → `TypeError: bad argument type for built-in operation`; `os.makedirs` → `SystemError: mkdir returned NULL`. `socket` not importable; `urllib` fails; no threads (Boyan253: "CPython sub-interpreter that blocks threads, sockets, and most os filesystem calls"). Available: `json`, `base64`, `binascii`, `math`, `time`, `pickle` (Flapi), `logging` (newer builds), `_io`. Boyan253 claims plain file **read/write of two fixed JSON files** still works on 25.2.5 (their "file-RPC") while flstudio-mcp says writes fail — **conflicting; VERIFY ON WINDOWS**. The manual's only related statement: custom modules can be placed in `C:\Program Files (x86)\Image-Line\Shared\Python\Lib` (older path) — **VERIFY current path** (`.../Image-Line/Shared/Python/Lib`).

### 3.3 Other FL MCP projects found (GitHub search 2026‑10‑09; READMEs in `SCRATCH/readmes/`)
| Repo | ★ | License | Transport / notes |
|---|---|---|---|
| karl-andres/fl-studio-mcp | 203 | MIT | MIDI trigger note + **JSON files** for commands (works on their FL; will break where file I/O is sandboxed), piano roll via ComposeWithLLM.pyscript + Ctrl+Alt+Y keystroke (foregrounds FL on Windows). FL 20.7+, loopMIDI, Python 3.12 pinned for rtmidi wheels. Limits: cannot load plugins, no pattern management. |
| rosasynthesiz/flstudio-mcp | 75 | MIT | §3.1 |
| **emmggew-svg/fl-mcp-2026** | 0 | MIT | "FL Studio 2026 port (v1.1.0) of rosasynthesiz/flstudio-mcp: capability layer, robust paths, backward compatible with FL 2025". Same architecture (`SCRATCH/readmes/emmggew-svg_fl-mcp-2026.md`). |
| veenastudio/flstudio-mcp | 105 | none | 2025, encodes notes into multiple MIDI note messages ("MIDI is only 7 bits"), live-records into piano roll. Primitive. |
| synopsys0/postfader-fl-studio-mcp | 7 | Apache‑2.0 | PyPI `postfader-fl-studio-mcp`, **FL Studio 2026 (26.1.3+)**, Windows+macOS, 85 tools, "Universal Bridge" controller script over one virtual port with same in/out port number, read-only until write mode enabled, readback after every write. |
| Boyan253/fl-studio-2025-ai-bridge | 2 | MIT | Fork of geezoria/FLStudioMCP for FL 25.2.5: file-RPC + MIDI-wake, `meta.exec` (remote exec of any API fn), notes baked into pyscript, **playlist clip placement via `transport.globalTransport(FPT_Copy/FPT_Paste)`**, note recording via `channels.triggerNote`. Claims `OnIdle` never fires on 25.2.5. |
| dawhubapp/flstudio-mcp | 1 | MIT | macOS, FL 25/26, live via IAC + offline `.flp` editing through their own TS parser **flpdiff** (`github.com/dawhubapp/flpdiff`) — a maintained alternative to PyFLP worth evaluating. Also `render_to_wav` by launching FL. |
| sadoway7/flaik | 4 | none | Claims **FL Studio 2026 has a built-in MCP server** used by the "Gopher" AI WebView2 panel (`https://gopher-fls.image-line.com/`, host object `script_handler`, JSON-RPC `tools/call`, 44 tools incl. add/remove effects, run piano-roll scripts). Reverse-engineered; ToS grey area; **unverified**. If true, FL 2026 offers a far richer control surface than MIDI scripting — **VERIFY**. |
| Realynx/Fl-MCP + FL-Automation | 1/2 | PolyForm NC | C#/.NET in-process plugin SDK ("FruityLink"), Windows x64, FL 26.1.3; non-commercial license. |
| calvinw/fl-studio-mcp (40★), geezoria/FLStudioMCP (7★), Zierax, szichedelic, ryurikoneko/DAWLoop, IzzoSol/FL-STUDIO-AI | | mostly MIT | variations of the same loopMIDI + device script pattern. |

---

## 4. FL Studio MIDI scripting facts (manual `SCRATCH/manual/midi_scripting.txt`, from https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/midi_scripting.htm; stubs docs `docs/midi_controller_scripting/script_files.md`, `script_metadata.md`, `tutorials/getting_started.md`)
- Location (Windows): `...Documents\Image-Line\FL Studio\Settings\Hardware\<devicename>\device_<devicename>.py` — folder name arbitrary; **`device_` prefix and `.py` mandatory**; spaces allowed. Script folder is on the script's `PYTHONPATH` (extra modules importable). Factory scripts live under the FL install (`...\FL Studio 2024\System\Config\Hardware`-ish; user scripts show as "**Name (user)**" in the Controller type list, in a "Scripts" section near the top per Boyan253).
- Metadata header (first lines): `# name=My Script` (**required**), `# url=https://forum.image-line.com/...`, `# receiveFrom=Other Script` (enables `device.dispatch` from that script), `# supportedDevices=Dev A,Dev B` (auto-link by device name — use your loopMIDI port name here so FL auto-assigns the script), `# supportedHardwareIds=00 20 29 ...` (auto-link by identity reply, prefix match).
- Enabling: *Options > MIDI Settings*: in **Output** list enable the port and give it a Port number; in **Input** list enable the port, set the **same** Port number, choose the script in **Controller type**. Restart FL after adding a new script; *View > Script output* has per-script tabs ("init ok" on success), a **Reload** button, an **Edit script** button, and an **Interpreter** tab (needs a script selected) for testing calls live.
- Callbacks: `OnInit, OnDeInit, OnMidiIn(ev) (filter only), OnMidiMsg(ev), OnSysEx(ev), OnNoteOn/Off, OnControlChange, OnProgramChange, OnPitchBend, OnKeyPressure, OnChannelPressure, OnMidiOutMsg, OnIdle, OnProjectLoad(status) v16, OnRefresh(flags), OnDoFullRefresh, OnUpdateBeatIndicator(v), OnDisplayZone, OnUpdateLiveMode, OnDirtyMixerTrack(i), OnDirtyChannel(i, flag) v16, OnFirstConnect v17, OnUpdateMeters, OnWaitingForInput, OnSendTempMsg(msg, dur)`. Manual: "OnSysEx — Called for SysEx messages that were not handled by OnMidiIn." flstudio-mcp found FL 25.2.5 delivers SysEx to **`OnSysEx`, not `OnMidiMsg`** (`FIX_REPORT.md` 3a) — implement both.
- Python: embedded CPython **3.12.1** on FL 24/25 (Flapi README, flstudio-mcp docs); older FL 20.x used 3.6/3.9 **(VERIFY for the target FL version)**. No pip, no sockets, no threads, no file writes (§3.2). Scripts run on FL's UI/main thread; `pmeFlags` gate "unsafe" operations (`TypeError("Operation unsafe at current time")`); `general.safeToEdit()` v29.
- SysEx size: **not documented by Image-Line**. Flapi: 1024-byte Windows buffers, split at 1000; flstudio-mcp: 1000 B OK, 2000 B dropped. Design for ≤ ~900 bytes per SysEx and chunk.
- `device.midiOutSysex(bytes)` requires the F0/F7 framing in the bytes; FL routes it to the **output device with the same Port number** as the script's input device.
- Piano roll scripts (`manual/pianoroll_scripting_api.htm`): user folder `...Documents\Image-Line\FL Studio\Settings\Piano roll scripts\` (sub-folders = menu categories), extension `.pyscript`, module `flpianoroll` (`flp.score`, `score.PPQ`, `Note()` with `.number/.time/.length/.velocity`, `score.addNote`, `score.clearNotes`, `ScriptDialog`); run only via Piano roll → Scripting menu or "Run last script again" (**Ctrl+Alt+Y** Windows / Cmd+Opt+Y macOS); file I/O blocked on 25.2.5.

---

## 5. PyFLP (GPL‑3.0) — offline .flp parsing

Source `SCRATCH/PyFLP` (demberto/PyFLP master f937126, **2023‑07‑11**; `dev` branch last "wip" 2023‑08‑08; PyPI latest **2.2.1 (2023‑06‑05)**; 39 open issues; not archived). License `LICENSE` = **GPL‑3.0** (copyleft — important if linked into a distributed Rust/Python product; use as a separate dev tool or reimplement the parser). Requires `construct`, `construct-typing`, `sortedcontainers`.

### 5.1 Version support / known limitations (GitHub issues, fetched 2026‑10‑09)
- **#183 (open, 2023‑11) Python 3.12 Enum issue** — `pyflp.parse` raises `TypeError: <enum 'EventEnum'> has no members` on 3.12 (and reported on 3.11.9 too); maintainer: "haven't tested on 3.12". Reproduced here; workaround in §5.3.
- **#200 (open, 2025‑08) Missing playlist data in FL Studio 2025 project files** — `PlaylistEvent` struct changed (struct sizes 32/60 no longer match); reporter proposes a new struct. **#199 (2025‑06) Playlist data structure error**, **#177 (2023) `UserWarning: Cannot parse event ArrangementID.Playlist as event size 2072 is not a multiple of struct size(s) [32, 60]`** (fixed by re-saving in FL 20). → playlist/arrangement items unreliable for FL 21+ files.
- **#203 (2026‑01) Unicode error when parsing flp**, **#201 (2025‑08) errors loading flp**, **#202 (2025‑11)** saved file shows "Buy FL Studio!" (likely trial-version save). **#197** Patcher unsupported; **#159** Fruity Parametric EQ 2 parser missing; **#193** mixer params typing. Only 9 native plugins parsed (BooBass, FruitKick, FruityBalance, FruityBloodOverdrive, FruityCenter, FruityFastDist, FruityNotebook2, FruitySend, FruitySoftClipper, FruityStereoEnhancer, Plucked, Soundgoodizer) + `VSTPlugin` (name, vendor, plugin_path, fourcc, guid, state blob). Unknown events are preserved as raw bytes so **save round-trips are byte-exact** (`tests/test_project.py::test_null_check`; confirmed below).
- README: "CPython 3.8+ / PyPy 3.8+", "It is advised to do a backup of your projects before doing any changes".
- Alternatives spotted: `dawhubapp/flpdiff` (TypeScript/bun parser+serializer used by dawhub's MCP, handles FL 25/26), forks `Meowrium/PyFLP` (4★), `leonmendum/PyFLP_FORK`, `westclox-star/FLPy` — none on PyPI; not evaluated.

### 5.2 Samples copied
PyFLP ships only two real project files (the rest are `.fst`/`.fsc` presets/scores and 22-byte corrupted stubs):
- `/home/tabor/github/flit/bridges/fl/flp/samples/FL 20.8.4.flp` (190,128 B; from `tests/assets/FL 20.8.4.flp`) — FL **20.8.4.2576**, the main test fixture (19 channels, 5 patterns, named inserts, VST slot).
- `/home/tabor/github/flit/bridges/fl/flp/samples/multi-channel.flp` (115,522 B; from `tests/assets/patterns/multi-channel.flp`) — FL **20.8.4.2576**, minimal (2 samplers, 1 pattern, 2 notes).
No FL 21/24/25 sample exists in the repo. (Realynx/Fl-MCP has FL 2026 `.flp` examples under `examples/*/*.flp` — PolyForm NC license, not copied.)

### 5.3 Exactly what worked (script `SCRATCH/scripts/run_pyflp.py`, venv python 3.12)
Monkeypatch required first (PyFLP #183):
```python
import enum, pyflp, pyflp._events as _ev
_Meta = type(_ev.EventEnum); _orig_call = _Meta.__call__
def _patched_call(cls, value, *args, **kwargs):
    if cls is _ev.EventEnum and not args and not kwargs:        # base enum has no members -> 3.12 TypeError
        member = cls._missing_(value)                           # subclass lookup, else pseudo-member
        if member is None: raise ValueError(f"{value!r} is not a valid {cls.__name__}")
        return member
    return _orig_call(cls, value, *args, **kwargs)
_Meta.__call__ = _patched_call
```
Attribute access that worked (quirks noted):
```python
p = pyflp.parse(path)                       # pyflp.parse(file: pathlib.Path | str) -> Project
p.version            # FLVersion(major=20, minor=8, build=4, patch=2576); str -> '20.8.4.2576'
p.ppq                # 96          p.tempo -> 69.42 (float BPM)      p.title / p.artists / p.genre / p.comments / p.url
p.channel_count      # 19 (stored count)   p.format -> FileFormat.Project (0)   p.created_on, p.time_spent, p.licensee, p.looped, p.pan_law
rack = p.channels    # ChannelRack; iterable -> Channel subclasses: Sampler | Instrument | Layer | Automation
for c in rack: c.iid, type(c).__name__, c.name, c.display_name, c.internal_name, c.volume (int 0..12800, 10000 default), c.pan (int 0..12800, 6400 center), c.enabled, c.color
   c.insert          # mixer insert index (Sampler/Instrument only; Layer/Automation raise AttributeError)
   c.plugin          # Instrument only: BooBass/FruitKick/Plucked/VSTPlugin/_PluginBase/None;  VSTPlugin.name -> 'Sylenth1'
rack.instruments / rack.samplers / rack.layers / rack.automations   # filtered iterators
pats = p.patterns    # Patterns; iterable; p.patterns.current -> Pattern or None
for pt in pats: pt.iid (1-based), pt.name (None if unnamed), pt.length (ticks; None if default), pt.color (RGBA or None), pt.notes (iterable), pt.timemarkers, pt.controllers
for n in pt.notes: n.position (ticks, PPQ-based: 384 = 1 bar @96), n.length (ticks, 0 = step-sequencer hit), n.key ('C5' str), n.velocity (0..128, 100 default),
                   n.rack_channel (channel iid), n.midi_channel, n.pan (0..128, 64 center), n.release, n.fine_pitch (120 center), n.slide (bool), n.group, n.mod_x, n.mod_y
mx = p.mixer         # Mixer; mx.max_inserts -> 127, mx.max_slots -> 10, mx.apdc
for ins in mx: ins.iid (-1 = Master, 0.. = inserts), ins.name (None if default), ins.volume (int, 12800 = 0 dB), ins.pan (int, 0 center), ins.enabled, ins.routes (list), ins.dock (InsertDock), ins.is_solo, ins.output, ins.input, ins.eq, ins.color
    for s in ins: s.index (0..9, or 256 in one quirk), s.name (user name or None), s.internal_name ('Fruity Wrapper' for VSTs), s.plugin (FruityBalance/... /VSTPlugin/None); s.plugin.name -> 'OTT' for VSTPlugin
arrs = p.arrangements   # Arrangements; arrs.current, arrs.time_signature, arrs.loop_pos
for a in arrs: a.iid, a.name, a.tracks (500 Track objects), a.timemarkers
   for t in a.tracks: t.iid, t.name, iter(t) -> PatternPLItem | ChannelPLItem with .position, .length, .group, .offsets, .pattern (PatternPLItem) / .channel (ChannelPLItem); .muted -> KeyError on FL 20.8.4 files
pyflp.save(p, out_path)  # pyflp.save(project: Project, file: pathlib.Path | str) -> None
```
Results:
| | FL 20.8.4.flp | multi-channel.flp |
|---|---|---|
| version / ppq / tempo | 20.8.4.2576 / 96 / 69.42 | 20.8.4.2576 / 96 / 140.0 |
| channels | 19: BooBass(Instrument), Instrument track(Sampler), Layer, Sampler, Colored, Automation Clip(Automation), VST2(Instrument, plugin VSTPlugin 'Sylenth1'), Audio Clip, Iconified(MIDI Out), Fruit Kick, Plucked!, 22in Kick, Zero Volume, Full Volume, 100% L, 100% R, Disabled, Locked, Zipped | 2 unnamed Samplers (name=None) |
| patterns / notes | 5 (MIDI, Default, Colored, Timemarkers, Selected); **48 notes** total (all in 'MIDI'); 'Timemarkers' has 5 timemarkers, length 1824 | 1 unnamed pattern; **2 notes** (C5, slide=True, rack_channel 0 and 1) |
| mixer | 127 inserts iterated (`max_inserts=127`), 24 named (Master, Audio track, Instrument track, Colored, ... 50ms input latency); slot plugins: insert 18 'Plugin Test' slots 0–5 = Fruity Balance, Fruity Send, Fruity Soft Clipper, Fruity Stereo Enhancer, Soundgoodizer, **VSTPlugin 'OTT'** (slot name 'OTT (failed to load)'); insert 19 slot 0 Fruity NoteBook 2; insert 17 slot index 256 Fruity NoteBook 2 (quirk) | 127 inserts, none named, no plugins |
| arrangements | 2 ('Just tracks' with items on tracks Audio track(16 ChannelPLItems of '22in Kick'), MIDI(4 PatternPLItems), Cut pattern, Automation; 'Just timemarkers' with 11 markers) | 1 ('Arrangement'), no items |
| parse warnings | 0 | 0 |
| `pyflp.save` round-trip byte-identical | **True** | **True** |

Quirks: `channel.volume/pan` and `insert.volume` are raw ints (12800 = 100%/0 dB, channel pan 6400 = center, insert pan 0 = center); `PLItem.muted` raises `KeyError` on FL 20.8.4 items (field only in the 60‑byte "new" struct); `pattern.length` is None unless set; `Mixer` iterates all 127 inserts even if unused (check `ins.name`/routes/slots to detect use).

---

## 6. Known limits (consolidated)
1. Only MIDI (SysEx) in/out of a controller script; no sockets/threads/subprocess; file writes blocked on FL 24/25 (reads disputed). One script runs per assigned MIDI input; handlers run synchronously on FL's UI thread — long handlers stall FL; FL may coalesce repeated state toggles within one tick.
2. Windows SysEx: keep each message ≤ ~1000 bytes (observed drop at ~1.5–2 KB; historical FL crash on overflow). Base64 payload costs +33%. Chunk + reassemble with a continuation flag; paginate list replies.
3. `midiOutSysex` output goes only to the output port whose Port number equals the script's input port number; both ports must be enabled correctly (enable RX only as input, TX only as output).
4. Virtual ports on Windows cannot be created from Python/rtmidi; loopMIDI (or similar) must run; MSIX-sandboxed clients may receive no MIDI → use a separate daemon process that owns the ports.
5. API has no: load plugin/audio, render/export, place playlist clips (workaround: `globalTransport(FPT_Copy/FPT_Paste)`), run a pyscript (needs user arm + Ctrl+Alt+Y keystroke), read project file path, read VST internal preset lists (wrapper exposes 128 "Prog n"), per-parameter change events (only dirty flags), `OnIdle` timing guarantees (~20 ms nominal; one report of it not firing on 25.2.5).
6. VST params: 4240 reported, real ones at low indices with empty names elsewhere; values normalized 0..1 with no generic unit mapping (use `getParamValueString`; calibrate per plugin). Mixer unity gain = 0.8, tempo raw = BPM×1000, `muteTrack(i,1)` unreliable, `REC_FromMIDI` rescales values.
7. Stubs are v37 (FL ~24) and unmaintained; manual is at v45 (FL 2026); several `midi.REC_Mixer_*/REC_Plug_*` constants missing from `midi/__init__.py` in stubs; `fl_classes` not importable in FL.
8. PyFLP: GPL‑3, unmaintained since 2023, broken on py3.12 without patch, playlist struct broken for FL 21+/2025 files, few native plugin parsers; byte-exact round-trip for unknown data. Reading FL 2025/2026 `.flp` reliably likely needs a new parser (see flpdiff).
9. Flapi: unmaintained; PyPI 1.0.1 and GitHub main are protocol-incompatible; uses pickle (code-exec surface); 0.1 s reply timeout is tight for heavy calls.

## 7. VERIFY ON WINDOWS (uncertainties to test first)
- [ ] Exact FL version/API version/Python version of the target install: in FL's Script output Interpreter run `import general, ui, sys; general.getVersion(); ui.getVersion(5); sys.version`.
- [ ] Which callback receives SysEx (`OnSysEx` vs `OnMidiMsg`) and whether `event.sysex` includes F0/F7 — log `list(event.sysex[:3])` and `[-1]`.
- [ ] Max SysEx size that survives loopMIDI → FL and FL → loopMIDI in both directions (probe 256/512/900/1000/1500/2000 bytes) and whether oversize is dropped or truncated; whether rapid back-to-back SysEx messages reorder.
- [ ] `OnIdle` cadence (log timestamps) and whether it fires at all when FL is unfocused/minimized; whether `OnRefresh` fires with `HW_Dirty_ControlValues` when a 3rd-party VST knob is moved by mouse, and with which flags for mixer fader moves.
- [ ] `plugins.getParamCount/Name/Value/setParamValue(..., slotIndex)` on a 3rd-party VST3 effect; whether `setParamValue` updates the plugin GUI immediately; whether `getParamValueString` works for your plugins.
- [ ] REC event addressing for effect params: `mixer.getTrackPluginId(t,s) + midi.REC_Plug_Plugin_First + p` vs `plugins.getParamValue(p,t,s)`; whether `processRECEvent(id, 0, REC_GetValue)` returns the value; `device.getLinkedValue/getLinkedParamName` on those ids; `mixer.getEventIDName`.
- [ ] File I/O inside controller scripts (read-only JSON read) on the target build — conflicting reports.
- [ ] Project name: contents of `ui.getProgTitle()` (does it include the `.flp` filename?) and availability of `general.getProjectTitle()` (v38+). Alternative: read `HKCU\Software\Image-Line\...` recent files or watch FL's window title via Win32.
- [ ] `muteTrack(index, value)` behaviour, `mixer.getCurrentTempo()` scale (×1000?), `trackCount()` including utility tracks, `getTrackPeaks` when stopped.
- [ ] Auto-assignment via `# supportedDevices=<loopMIDI port name>` to avoid the manual Controller-type step; behaviour of FL's "Reload script" vs full restart.
- [ ] Whether FL 2026 really exposes a built-in MCP/`script_handler` (flaik claim) — could obsolete much of the MIDI bridge.
- [ ] PyFLP on a real FL 2024/2025 `.flp` from the user: parse warnings, playlist items, unicode errors; byte-exact round trip.

## 8. Source index
- Flapi: `SCRATCH/Flapi/{README.md,Protocol.md,Unmaintained.md,LICENSE.md,flapi/_consts.py,flapi/flapi_msg.py,flapi/client/comms.py,flapi/device_flapi_receive.py}`; PyPI 1.0.1: `SCRATCH/flapi_pip/flapi/{__comms.py,__enable.py,__decorate.py,server/device_flapi_receive.py,server/device_flapi_respond.py,server/flapi_response.py,server/consts.py,server/capout.py,cli/install.py,cli/util.py}`; issues https://github.com/MaddyGuthridge/Flapi/issues/{11,32,37}.
- Stubs: `SCRATCH/FL-Studio-API-Stubs/src/midi_controller_scripting/{device/__device.py,device/__fl.py,plugins/__init__.py,mixer/__tracks.py,general/__fl_state.py,ui/__state.py,callbacks/__init__.py,fl_classes/__init__.py,midi/__on_refresh_flags.py,midi/__rec_events/{ranges,channel_properties,mixer_properties,global_properties,process_flags,special}.py}`, docs `docs/midi_controller_scripting/{script_files.md,script_metadata.md,tutorials/getting_started.md,tutorials/event_mapping.md,tutorials/time_units.md}`; introspection dumps: `SCRATCH/scripts/introspect_stubs.py`, `introspect2.py`.
- flstudio-mcp: `SCRATCH/flstudio-mcp/{README.md,ROADMAP.md,fl_controller/FLStudioMCP/device_FLStudioMCP.py,src/fl_studio_mcp/{protocol.py,connection.py,pyscript_gen.py,pyscript_trigger.py,tools/plugin.py},docs/{architecture.md,CHANGELOG.md,FIX_REPORT.md,PHASE1A_REPORT.md,PHASE1B_REPORT.md,VST_PROBE_FINDING.md,SERUM_PROBE_FINDING.md,NOTE_BRIDGE_SETUP.md,ARRANGEMENT_FINDING.md,MIXING_ROUTING_REPORT.md},scripts/install_windows.bat}`.
- Other MCP READMEs: `SCRATCH/readmes/*.md` (karl-andres, emmggew-svg_fl-mcp-2026, veenastudio, synopsys0_postfader, dawhubapp, sadoway7_flaik, Boyan253, Realynx).
- FL manual text: `SCRATCH/manual/midi_scripting.txt`, `SCRATCH/manual/pianoroll_scripting_api.htm` (https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/midi_scripting.htm, .../pianoroll_scripting_api.htm).
- PyFLP: `SCRATCH/PyFLP/{README.md,CHANGELOG.md,LICENSE,tests/conftest.py,tests/test_project.py,tests/test_pattern.py}`, venv `site-packages/pyflp/{__init__.py,_events.py}`; run script `SCRATCH/scripts/run_pyflp.py`; issues https://github.com/demberto/PyFLP/issues/{183,200,199,177,203}.