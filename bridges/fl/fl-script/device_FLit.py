# name=FLit
# url=https://github.com/taborgreat/flit
# supportedDevices=FLit In
"""FLit controller script for FL Studio.

Copy this folder to
  Documents\\Image-Line\\FL Studio\\Settings\\Hardware\\FLit\\device_FLit.py
and in Options > MIDI Settings enable the loopMIDI ports "FLit In" (input,
Controller type: FLit (user), Port 42) and "FLit Out" (output, Port 42).

It speaks SysEx to the flit-agent (see ../src/sysex.rs for the protocol).
Everything FL-specific lives here or in the Rust bridge; the FLit core
never knows this file exists.

Sandbox rules (FL 24/25): no sockets, no threads, no file writes, everything
runs on FL's UI thread. Keep every handler tiny; the agent does the thinking.
"""

import base64
import json
import time

PROTOCOL = 1
HEADER = bytes([0x7D, ord("F"), ord("L"), ord("i"), ord("t")])
DIR_REQUEST, DIR_REPLY, DIR_EVENT = 1, 2, 3
CHUNK_PAYLOAD = 600  # VERIFY ON WINDOWS: must match CHUNK_PAYLOAD in sysex.rs
PARAM_PAGE_MAX = 64
MAX_SLOTS = 10

# ----- FL modules (missing when unit-tested on Linux with the stubs) -----
try:
    import channels  # type: ignore
    import device  # type: ignore
    import general  # type: ignore
    import midi  # type: ignore
    import mixer  # type: ignore
    import plugins  # type: ignore
    import transport  # type: ignore
    import ui  # type: ignore
    IN_FL = True
except ImportError:  # unit tests outside FL
    channels = device = general = midi = mixer = plugins = transport = ui = None
    IN_FL = False


class Codec:
    """Encode/decode frames. Mirrors sysex.rs exactly."""

    def __init__(self):
        self.partial = {}

    @staticmethod
    def encode(direction, req_id, obj):
        payload = base64.b64encode(json.dumps(obj, separators=(",", ":")).encode("utf-8"))
        chunks = [payload[i:i + CHUNK_PAYLOAD] for i in range(0, len(payload), CHUNK_PAYLOAD)] or [b""]
        out = []
        for i, c in enumerate(chunks):
            more = 1 if i + 1 < len(chunks) else 0
            out.append(bytes([0xF0]) + HEADER + bytes([direction, (req_id >> 7) & 0x7F, req_id & 0x7F, more]) + c + bytes([0xF7]))
        return out

    def push(self, msg):
        """Feed one SysEx message (with or without F0/F7). Returns
        (direction, id, obj) when a frame is complete, else None."""
        m = bytes(msg)
        if m[:1] == b"\xF0":
            m = m[1:]
        if m[-1:] == b"\xF7":
            m = m[:-1]
        if len(m) < len(HEADER) + 4 or m[:len(HEADER)] != HEADER:
            return None
        rest = m[len(HEADER):]
        direction, hi, lo, more = rest[0], rest[1], rest[2], rest[3]
        if direction not in (DIR_REQUEST, DIR_REPLY, DIR_EVENT) or hi > 0x7F or lo > 0x7F or more > 1:
            return None
        req_id = (hi << 7) | lo
        key = (direction, req_id)
        buf = self.partial.get(key, b"") + rest[4:]
        if len(buf) > 4 * 1024 * 1024:
            self.partial.pop(key, None)
            return None
        if more:
            self.partial[key] = buf
            return None
        self.partial.pop(key, None)
        try:
            return direction, req_id, json.loads(base64.b64decode(buf, validate=True).decode("utf-8"))
        except Exception:
            return None


codec = Codec()
_last_heartbeat = time.monotonic()
_dirty_tracks = []
_dirty_channels = []
_dirty_flags = 0
_dirty_pending = False
_idle_count = 0


# ----- sending -----
def _send(direction, req_id, obj):
    if not IN_FL:
        return SENT.append((direction, req_id, obj))
    fn = getattr(device, "midiOutSysex", None) or getattr(device, "midiOutSysEx", None)
    if fn is None:
        return
    for m in Codec.encode(direction, req_id, obj):
        fn(m)  # VERIFY ON WINDOWS: must include F0/F7; routed to the output port with the same port number


SENT = []  # test capture when not in FL


def _event(obj):
    _send(DIR_EVENT, 0, obj)


def _reply(req_id, obj):
    _send(DIR_REPLY, req_id, obj)


def _err(req_id, text):
    _reply(req_id, {"ok": False, "error": str(text)})


# ----- FL state helpers -----
def _tempo():
    t = mixer.getCurrentTempo()
    # VERIFY ON WINDOWS: raw value is BPM*1000 on FL 25 (flstudio-mcp)
    return t / 1000.0 if t > 1000 else float(t)


def _pos_ticks():
    try:
        return int(transport.getSongPos(midi.SONGLENGTH_ABSTICKS))  # VERIFY ON WINDOWS: absolute ticks in song mode
    except Exception:
        return int(mixer.getSongTickPos())


def _ppq():
    try:
        return int(general.getRecPPQ())
    except Exception:
        return 96


def _track(i):
    slots = []
    for s in range(MAX_SLOTS):
        try:
            if plugins.isValid(i, s):
                slots.append([s, plugins.getPluginName(i, s)])
        except Exception:
            pass
    return {
        "index": i,
        "name": mixer.getTrackName(i),
        "volume": float(mixer.getTrackVolume(i)),  # 0.8 = unity. VERIFY ON WINDOWS
        "pan": float(mixer.getTrackPan(i)),
        "mute": bool(mixer.isTrackMuted(i)),
        "slots": slots,
    }


def _channel(i):
    """A channel-rack channel (global index): its generator plugin, which
    insert it goes to, and its own volume/pan/mute."""
    def g(fn, default):
        try:
            return fn()
        except Exception:
            return default
    plugin = None
    try:
        if plugins.isValid(i, -1, True):
            plugin = plugins.getPluginName(i, -1, False, True)
    except Exception:
        plugin = None
    return {
        "index": i,
        "name": g(lambda: str(channels.getChannelName(i, True)), ""),
        "plugin": plugin,
        "insert": g(lambda: int(channels.getTargetFxTrack(i)), 0),
        "volume": g(lambda: float(channels.getChannelVolume(i, False, True)), 0.78125),  # 0.78125 = default/unity
        "pan": g(lambda: float(channels.getChannelPan(i, True)), 0.0),
        "mute": g(lambda: bool(channels.isChannelMuted(i, True)), False),
    }


def _snapshot():
    n = mixer.trackCount()
    # Verified on FL 24.1.1: trackCount() = 127 (Master, 125 inserts, one utility track).
    tracks = [_track(i) for i in range(0, max(1, n - 1))]
    try:
        n_ch = int(channels.channelCount(True))
    except Exception:
        n_ch = 0
    chans = [_channel(i) for i in range(n_ch)]
    return {"ok": True, "tempo": _tempo(), "playing": bool(transport.isPlaying()), "pos_ticks": _pos_ticks(), "ppq": _ppq(), "tracks": tracks, "channels": chans}


def _params(track, slot, start, count):
    # slot -1 = the channel-rack generator of channel `track` (global index)
    glob = slot == -1
    count = max(1, min(int(count), PARAM_PAGE_MAX))
    total = plugins.getParamCount(track, slot, glob)
    out = []
    i = start
    end = min(total, start + count)
    while i < end:
        name = plugins.getParamName(i, track, slot, glob)
        if name:  # VST wrappers report 4240 params; unused ones have empty names
            try:
                text = plugins.getParamValueString(i, track, slot, 0, glob)
            except Exception:
                text = ""
            out.append({"index": i, "name": name, "value": float(plugins.getParamValue(i, track, slot, glob)), "text": text})
        i += 1
    return {"ok": True, "track": track, "slot": slot, "total": int(total), "next": int(end), "params": out}


def _focus():
    try:
        if ui.getFocused(midi.widPluginEffect) or ui.getFocused(midi.widPluginGenerator) or ui.getFocused(midi.widPlugin):
            fid = ui.getFocusedFormID()
            if fid >= 0:
                track, slot = fid >> 22, (fid >> 16) & 0x3F  # track*4194304 + slot*65536. VERIFY ON WINDOWS
                return {"focus": "plugin", "track": track, "slot": slot}
        if ui.getFocused(midi.widMixer):
            return {"focus": "mixer", "track": -1, "slot": -1}
        if ui.getFocused(midi.widPianoRoll):
            return {"focus": "piano_roll", "track": -1, "slot": -1}
        if ui.getFocused(midi.widPlaylist):
            return {"focus": "playlist", "track": -1, "slot": -1}
        if ui.getFocused(midi.widChannelRack):
            return {"focus": "channel_rack", "track": -1, "slot": -1}
    except Exception:
        pass
    return {"focus": "other", "track": -1, "slot": -1}


def _changed_flag():
    try:
        return int(general.getChangedFlag())
    except Exception:
        return -1


def _title():
    try:
        return str(ui.getProgTitle())
    except Exception:
        return ""


def _undo_history():
    # FL's own undo history: it already decides what counts as one edit, so
    # a change in (count, position) is a clean "a real act just happened"
    # signal that can catch edits the dirty flags miss. VERIFY ON WINDOWS:
    # what detail it exposes (see HANDOFF.md).
    try:
        return int(general.getUndoHistoryCount()), int(general.getUndoHistoryPos())
    except Exception:
        return -1, -1


def _heartbeat():
    global _last_heartbeat
    _last_heartbeat = time.monotonic()
    try:
        undo_count, undo_pos = _undo_history()
        _event({"ev": "heartbeat", "playing": bool(transport.isPlaying()), "tempo": _tempo(), "pos_ticks": _pos_ticks(), "changed": _changed_flag(), "title": _title(), "undo_count": undo_count, "undo_pos": undo_pos})
    except Exception:
        pass


def _heartbeat_if_stale():
    # Fallback when OnIdle does not fire (reported on one FL 25 build):
    # other callbacks keep the agent informed at least once a second.
    if time.monotonic() - _last_heartbeat > 1.0:
        _heartbeat()


def _versions():
    api = general.getVersion() if general else -1
    try:
        fl = ui.getVersion(4)
    except Exception:
        fl = "?"
    import sys
    return api, str(fl), sys.version.split()[0]


# ----- request handling -----
def handle(req_id, req):
    cmd = req.get("cmd")
    try:
        if cmd == "hello":
            api, fl, py = _versions()
            _reply(req_id, {"ok": True, "api": api, "fl": fl, "python": py, "ppq": _ppq()})
        elif cmd == "snapshot":
            _reply(req_id, _snapshot())
        elif cmd == "params":
            _reply(req_id, _params(int(req["track"]), int(req["slot"]), int(req.get("from", 0)), int(req.get("count", 48))))
        elif cmd == "set_param":
            # VERIFY ON WINDOWS: setParamValue on 3rd-party VST3 effects updates the plugin GUI
            s = int(req["slot"])
            plugins.setParamValue(float(req["value"]), int(req["param"]), int(req["track"]), s, 0, s == -1)
            _reply(req_id, {"ok": True})
        elif cmd == "set_chan_volume":
            channels.setChannelVolume(int(req["chan"]), float(req["value"]), 0, True)
            _reply(req_id, {"ok": True})
        elif cmd == "set_chan_pan":
            channels.setChannelPan(int(req["chan"]), float(req["value"]), 0, True)
            _reply(req_id, {"ok": True})
        elif cmd == "set_chan_mute":
            c = int(req["chan"])
            if bool(channels.isChannelMuted(c, True)) != bool(req["value"]):
                channels.muteChannel(c, -1, True)
            _reply(req_id, {"ok": True})
        elif cmd == "set_slot_enabled":
            # The nearest thing to removing a plugin that FL's API allows:
            # the slot's enable switch. REC_Plug_Mute 1 = bypassed.
            # VERIFY ON WINDOWS: value scale of this REC event (0/1 assumed).
            t, s = int(req["track"]), int(req["slot"])
            rec = getattr(midi, "REC_Plug_Mute", 536878848) + mixer.getTrackPluginId(t, s)
            r = general.processRECEvent(rec, 0 if bool(req["value"]) else 1, midi.REC_UpdateValue | midi.REC_UpdateControl)
            _reply(req_id, {"ok": True, "result": int(r) if isinstance(r, int) else -1})
        elif cmd == "save":
            # FL saves the project itself (Save As dialog when untitled); the
            # heartbeat's changed flag reports when the file is written.
            transport.globalTransport(getattr(midi, "FPT_Save", 92), 1)
            _reply(req_id, {"ok": True})
        elif cmd == "set_volume":
            mixer.setTrackVolume(int(req["track"]), float(req["value"]))
            _reply(req_id, {"ok": True})
        elif cmd == "set_pan":
            mixer.setTrackPan(int(req["track"]), float(req["value"]))
            _reply(req_id, {"ok": True})
        elif cmd == "set_mute":
            t = int(req["track"])
            if bool(mixer.isTrackMuted(t)) != bool(req["value"]):
                mixer.muteTrack(t)  # bare toggle: muteTrack(t, 1) unreliable on FL 25. VERIFY ON WINDOWS
            _reply(req_id, {"ok": True})
        elif cmd == "set_tempo":
            # value = BPM*1000; do NOT add REC_FromMIDI. VERIFY ON WINDOWS
            general.processRECEvent(midi.REC_Tempo, int(round(float(req["bpm"]) * 1000)), midi.REC_UpdateValue | midi.REC_UpdateControl)
            _reply(req_id, {"ok": True})
        elif cmd == "play":
            if not transport.isPlaying():
                transport.start()  # start() is play/pause toggle
            _reply(req_id, {"ok": True})
        elif cmd == "stop":
            transport.stop()
            _reply(req_id, {"ok": True})
        elif cmd == "set_pos":
            transport.setSongPos(int(req["ticks"]), midi.SONGLENGTH_ABSTICKS)
            _reply(req_id, {"ok": True})
        elif cmd == "focus":
            f = _focus()
            f["ok"] = True
            _reply(req_id, f)
        elif cmd == "title":
            _reply(req_id, {"ok": True, "title": ui.getProgTitle()})  # VERIFY ON WINDOWS: contains "name.flp"?
        else:
            _err(req_id, "unknown command %r" % (cmd,))
    except Exception as e:  # never let an exception escape into FL
        _err(req_id, "%s: %s" % (type(e).__name__, e))


def _handle_sysex(event):
    try:
        frame = codec.push(event.sysex)
    except Exception:
        return False
    if frame is None:
        return False
    direction, req_id, obj = frame
    if direction != DIR_REQUEST or not isinstance(obj, dict):
        return True
    handle(req_id, obj)
    return True


# ----- FL callbacks -----
def OnInit():
    api, fl, py = _versions()
    _event({"ev": "hello", "api": api, "fl": fl, "python": py})
    if device is not None and hasattr(device, "setHasMeters"):
        pass  # meters not needed


def OnDeInit():
    pass


def OnSysEx(event):
    # FL 21+/API v40 delivers SysEx here. VERIFY ON WINDOWS
    event.handled = _handle_sysex(event)


def OnMidiMsg(event):
    # Some builds deliver SysEx here instead. VERIFY ON WINDOWS
    if getattr(event, "status", 0) == 0xF0 or getattr(event, "sysex", None):
        event.handled = _handle_sysex(event)


def _flush_dirty():
    global _dirty_pending, _dirty_tracks, _dirty_channels, _dirty_flags
    if _dirty_pending:
        _dirty_pending = False
        _event({"ev": "dirty", "flags": _dirty_flags, "tracks": sorted(set(_dirty_tracks)), "channels": sorted(set(_dirty_channels))})
        _dirty_tracks = []
        _dirty_channels = []
        _dirty_flags = 0


def OnIdle():
    # ~20 ms nominal (one report says never on FL 25.2.5: VERIFY ON WINDOWS).
    global _idle_count
    _idle_count += 1
    _flush_dirty()
    if _idle_count % 25 == 0:  # ~every 500 ms
        _heartbeat()


def OnRefresh(flags):
    # HW_Dirty_ControlValues 4096, HW_Dirty_RemoteLinkValues 512, HW_Dirty_Mixer_Controls 4,
    # HW_Dirty_Names 16384. VERIFY ON WINDOWS which fire for VST knob moves.
    global _dirty_pending, _dirty_flags
    interesting = 4096 | 512 | 4 | 16384 | 2048
    if flags & interesting:
        _dirty_flags |= flags
        _dirty_pending = True
    _heartbeat_if_stale()


def OnUpdateBeatIndicator(value):
    # Fires on every beat while playing: another chance to flush when OnIdle is quiet.
    _flush_dirty()
    _heartbeat_if_stale()


def OnDirtyMixerTrack(index):
    global _dirty_pending
    _dirty_tracks.append(int(index))
    _dirty_pending = True


def OnDirtyChannel(index, flag=0):
    global _dirty_pending
    _dirty_channels.append(int(index))
    _dirty_pending = True


def OnProjectLoad(status):
    if status == 100:  # PL_LoadOk
        try:
            title = ui.getProgTitle()
        except Exception:
            title = ""
        _event({"ev": "project_loaded", "title": title})
