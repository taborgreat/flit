"""Unit tests for device_FLit.py outside FL. Run with the venv python:
    python -I -m unittest fl-script/test_device_flit.py
Uses fake FL modules so no FL Studio is needed. Also writes the test
vectors the Rust side checks against (see sysex.rs tests).
"""
import json
import os
import sys
import unittest
import importlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import device_FLit as flit  # noqa: E402


class FakeMixer:
    def __init__(self):
        self.vol = {0: 0.8, 1: 0.8, 2: 0.5}
        self.pan = {0: 0.0, 1: 0.0, 2: -0.25}
        self.mute = {0: False, 1: False, 2: True}
        self.names = {0: "Master", 1: "Drums", 2: "Bass"}
    def trackCount(self): return 4  # master + 2 inserts + utility
    def getTrackName(self, i): return self.names.get(i, "")
    def getTrackVolume(self, i, mode=0): return self.vol.get(i, 0.8)
    def setTrackVolume(self, i, v, pickup=0): self.vol[i] = v
    def getTrackPan(self, i): return self.pan.get(i, 0.0)
    def setTrackPan(self, i, v, pickup=0): self.pan[i] = v
    def isTrackMuted(self, i): return self.mute.get(i, False)
    def muteTrack(self, i, value=-1): self.mute[i] = not self.mute.get(i, False)
    def getCurrentTempo(self, asInt=False): return 128000  # BPM*1000 as on FL 25
    def getTrackPluginId(self, i, slot): return ((i << 6) + slot) << 16
    def getSongTickPos(self, mode=0): return 192


class FakePlugins:
    def __init__(self):
        self.params = {(1, 0): [("Threshold", 0.5), ("Ratio", 0.25)] + [("", 0.0)] * 4238, (1, -1): [("Cutoff", 0.3)] + [("", 0.0)] * 4239}
        self.names = {(1, 0): "Fruity Limiter", (1, -1): "Sytrus"}
    def isValid(self, i, s=-1, g=False): return (i, s) in self.names
    def getPluginName(self, i, s=-1, userName=False, g=False): return self.names[(i, s)]
    def getParamCount(self, i, s=-1, g=False): return len(self.params[(i, s)])
    def getParamName(self, p, i, s=-1, g=False): return self.params[(i, s)][p][0]
    def getParamValue(self, p, i, s=-1, g=False): return self.params[(i, s)][p][1]
    def getParamValueString(self, p, i, s=-1, pickup=0, g=False): return "%.1f dB" % (self.params[(i, s)][p][1] * -30)
    def setParamValue(self, v, p, i, s=-1, pickup=0, g=False): self.params[(i, s)][p] = (self.params[(i, s)][p][0], v)


class FakeChannels:
    def __init__(self):
        self.names = {0: "Kick", 1: "Sytrus"}
        self.vol = {0: 0.78125, 1: 0.5}
        self.pan = {0: 0.0, 1: 0.25}
        self.mute = {0: False, 1: False}
        self.insert = {0: 1, 1: 2}
    def channelCount(self, g=False): return 2
    def getChannelName(self, i, g=False): return self.names[i]
    def getTargetFxTrack(self, i): return self.insert[i]
    def getChannelVolume(self, i, mode=False, g=False): return self.vol[i]
    def getChannelPan(self, i, g=False): return self.pan[i]
    def isChannelMuted(self, i, g=False): return self.mute[i]
    def setChannelVolume(self, i, v, pickup=0, g=False): self.vol[i] = v
    def setChannelPan(self, i, v, pickup=0, g=False): self.pan[i] = v
    def muteChannel(self, i, value=-1, g=False): self.mute[i] = not self.mute[i]


class FakeTransport:
    def __init__(self): self.playing = False; self.commands = []
    def globalTransport(self, command, value, pme=2, flags=15): self.commands.append((command, value)); return 0
    def isPlaying(self): return self.playing
    def start(self): self.playing = not self.playing
    def stop(self): self.playing = False
    def getSongPos(self, mode=-1): return 192
    def setSongPos(self, p, mode=-1): self.pos = p


class FakeGeneral:
    def __init__(self): self.rec = []
    def getRecPPQ(self): return 96
    def getVersion(self): return 40
    def processRECEvent(self, eid, value, flags): self.rec.append((eid, value, flags)); return 0


class FakeMidi:
    SONGLENGTH_ABSTICKS = 2
    REC_Tempo = 1073741829
    REC_UpdateValue = 1
    REC_UpdateControl = 16
    FPT_Save = 92
    REC_Plug_Mute = 536878848
    widMixer, widChannelRack, widPlaylist, widPianoRoll, widBrowser, widPlugin, widPluginEffect, widPluginGenerator = range(8)


class FakeUi:
    def getFocused(self, i): return i == FakeMidi.widMixer
    def getFocusedFormID(self): return -1
    def getProgTitle(self): return "FL Studio 2025 - my song.flp"
    def getVersion(self, mode=0): return "Producer Edition v25.2.5 [build 5319]"


class Ev:
    def __init__(self, sysex): self.sysex = sysex; self.handled = False; self.status = 0xF0


def install_fakes():
    flit.mixer, flit.plugins, flit.transport, flit.general, flit.midi, flit.ui = FakeMixer(), FakePlugins(), FakeTransport(), FakeGeneral(), FakeMidi(), FakeUi()
    flit.channels = FakeChannels()
    flit.IN_FL = False  # the venv has the FL stubs installed; capture into SENT instead
    flit.SENT.clear()
    flit.codec = flit.Codec()


_next_id = [100]


def request(obj, req_id=None):
    if req_id is None:
        _next_id[0] += 1
        req_id = _next_id[0]
    for m in flit.Codec.encode(flit.DIR_REQUEST, req_id, obj):
        ev = Ev(m)
        flit.OnSysEx(ev)
        assert ev.handled
    return [o for (d, i, o) in flit.SENT if d == flit.DIR_REPLY and i == req_id]


class CodecTests(unittest.TestCase):
    def test_roundtrip_and_chunking(self):
        c = flit.Codec()
        obj = {"cmd": "params", "track": 1, "slot": 0, "from": 0, "count": 48}
        msgs = flit.Codec.encode(flit.DIR_REQUEST, 0x1234, obj)
        self.assertEqual(len(msgs), 1)
        self.assertEqual(msgs[0][0], 0xF0)
        self.assertEqual(msgs[0][-1], 0xF7)
        self.assertTrue(all(b < 0x80 for b in msgs[0][1:-1]))
        self.assertEqual(c.push(msgs[0]), (flit.DIR_REQUEST, 0x1234, obj))
        big = {"ok": True, "params": [{"index": i, "name": "Param %d long name" % i, "value": i / 100.0, "text": "x"} for i in range(150)]}
        msgs = flit.Codec.encode(flit.DIR_REPLY, 9, big)
        self.assertGreater(len(msgs), 5)
        self.assertTrue(all(len(m) <= flit.CHUNK_PAYLOAD + 11 for m in msgs))
        got = [c.push(m) for m in msgs]
        self.assertEqual([g for g in got if g is not None], [(flit.DIR_REPLY, 9, big)])
        self.assertEqual(c.partial, {})

    def test_garbage(self):
        c = flit.Codec()
        for m in (b"", b"\xF0\xF7", b"\xF0\x7DFLit\x09\x00\x00\x00\xF7", bytes([0xFF] * 50), b"\xF0\x7DFLit\x02\x00\x01\x00!!!!\xF7"):
            self.assertIsNone(c.push(m))
        self.assertEqual(c.partial, {})

    def test_vectors_for_rust(self):
        """Write vectors the Rust test reads, so both codecs stay in lockstep."""
        obj = {"cmd": "set_param", "track": 3, "slot": 1, "param": 17, "value": 0.42}
        msgs = flit.Codec.encode(flit.DIR_REQUEST, 0x1234, obj)
        big = {"ev": "log", "text": "x" * 1500}
        chunks = flit.Codec.encode(flit.DIR_EVENT, 0, big)
        out = {"small": [list(m) for m in msgs], "small_json": json.dumps(obj, separators=(",", ":")), "chunked": [list(m) for m in chunks], "chunked_json": json.dumps(big, separators=(",", ":"))}
        with open(os.path.join(HERE, "codec_vectors.json"), "w") as f:
            json.dump(out, f)


class HandlerTests(unittest.TestCase):
    def setUp(self):
        install_fakes()

    def test_hello_and_snapshot(self):
        (r,) = request({"cmd": "hello", "protocol": 1})
        self.assertTrue(r["ok"]); self.assertEqual(r["api"], 40); self.assertEqual(r["ppq"], 96)
        (s,) = request({"cmd": "snapshot"})
        self.assertEqual(s["tempo"], 128.0)
        self.assertEqual([t["name"] for t in s["tracks"]], ["Master", "Drums", "Bass"])
        self.assertEqual(s["tracks"][1]["slots"], [[0, "Fruity Limiter"]])
        self.assertTrue(s["tracks"][2]["mute"])
        self.assertEqual([c["name"] for c in s["channels"]], ["Kick", "Sytrus"])
        self.assertEqual(s["channels"][1]["plugin"], "Sytrus")
        self.assertIsNone(s["channels"][0]["plugin"], "a sampler is not a plugin")
        self.assertEqual((s["channels"][1]["insert"], s["channels"][1]["volume"], s["channels"][1]["pan"]), (2, 0.5, 0.25))

    def test_params_paginate_and_skip_unnamed(self):
        (p,) = request({"cmd": "params", "track": 1, "slot": 0, "from": 0, "count": 48})
        self.assertEqual(p["total"], 4240); self.assertEqual(p["next"], 48)
        self.assertEqual([x["name"] for x in p["params"]], ["Threshold", "Ratio"])
        self.assertEqual(p["params"][0]["text"], "-15.0 dB")
        (p2,) = request({"cmd": "params", "track": 1, "slot": 0, "from": 4200, "count": 999})
        self.assertEqual(p2["next"], 4240); self.assertEqual(p2["params"], [])
        # slot -1 = the channel's generator
        (p3,) = request({"cmd": "params", "track": 1, "slot": -1, "from": 0, "count": 8})
        self.assertEqual([x["name"] for x in p3["params"]], ["Cutoff"])

    def test_setters(self):
        request({"cmd": "set_param", "track": 1, "slot": 0, "param": 1, "value": 0.9})
        self.assertAlmostEqual(flit.plugins.getParamValue(1, 1, 0), 0.9)
        request({"cmd": "set_volume", "track": 2, "value": 0.8}); self.assertEqual(flit.mixer.vol[2], 0.8)
        request({"cmd": "set_param", "track": 1, "slot": -1, "param": 0, "value": 0.6}); self.assertAlmostEqual(flit.plugins.getParamValue(0, 1, -1), 0.6)
        request({"cmd": "set_chan_volume", "chan": 1, "value": 0.7}); self.assertEqual(flit.channels.vol[1], 0.7)
        request({"cmd": "set_chan_pan", "chan": 1, "value": -0.5}); self.assertEqual(flit.channels.pan[1], -0.5)
        request({"cmd": "set_chan_mute", "chan": 1, "value": True}); self.assertTrue(flit.channels.mute[1])
        request({"cmd": "set_chan_mute", "chan": 1, "value": True}); self.assertTrue(flit.channels.mute[1], "no toggle when already right")
        request({"cmd": "save"}); self.assertEqual(flit.transport.commands, [(92, 1)])
        flit.general.rec.clear()
        request({"cmd": "set_slot_enabled", "track": 1, "slot": 0, "value": False})
        self.assertEqual(flit.general.rec, [(536878848 + ((1 << 6) + 0 << 16), 1, 17)], "bypass = mute 1 on the slot's REC id")
        request({"cmd": "set_slot_enabled", "track": 1, "slot": 0, "value": True})
        self.assertEqual(flit.general.rec[-1], (536878848 + ((1 << 6) + 0 << 16), 0, 17))
        request({"cmd": "set_mute", "track": 2, "value": False}); self.assertFalse(flit.mixer.mute[2])
        request({"cmd": "set_mute", "track": 2, "value": False}); self.assertFalse(flit.mixer.mute[2], "no toggle when already right")
        flit.general.rec.clear()
        request({"cmd": "set_tempo", "bpm": 90.5})
        self.assertEqual(flit.general.rec, [(1073741829, 90500, 17)])
        request({"cmd": "play"}); self.assertTrue(flit.transport.playing)
        request({"cmd": "play"}); self.assertTrue(flit.transport.playing, "play is idempotent, not a toggle")
        request({"cmd": "stop"}); self.assertFalse(flit.transport.playing)
        (t,) = request({"cmd": "title"}); self.assertIn("my song.flp", t["title"])
        (f,) = request({"cmd": "focus"}); self.assertEqual(f["focus"], "mixer")

    def test_errors_never_escape(self):
        (e,) = request({"cmd": "nope"})
        self.assertFalse(e["ok"])
        (e2,) = request({"cmd": "set_param", "track": "x"})
        self.assertFalse(e2["ok"]); self.assertIn("Error", e2["error"])
        ev = Ev(b"\xF0\x01\x02\xF7")
        flit.OnSysEx(ev)
        self.assertFalse(ev.handled)

    def test_dirty_and_heartbeat_events(self):
        flit.OnRefresh(4096)
        flit.OnDirtyMixerTrack(2)
        flit.OnDirtyChannel(1)
        flit.OnIdle()
        evs = [o for (d, i, o) in flit.SENT if d == flit.DIR_EVENT]
        self.assertEqual(evs, [{"ev": "dirty", "flags": 4096, "tracks": [2], "channels": [1]}])
        flit.OnRefresh(1)  # uninteresting flag
        flit.SENT.clear()
        for _ in range(25):
            flit.OnIdle()
        evs = [o for (d, i, o) in flit.SENT if d == flit.DIR_EVENT]
        self.assertEqual(len(evs), 1); self.assertEqual(evs[0]["ev"], "heartbeat"); self.assertEqual(evs[0]["tempo"], 128.0)
        self.assertEqual(evs[0]["changed"], -1, "fake general has no getChangedFlag: reported as -1, never raises")
        self.assertIn("my song.flp", evs[0]["title"])
        self.assertEqual((evs[0]["undo_count"], evs[0]["undo_pos"]), (-1, -1), "fake general has no undo history: -1, never raises")
        flit.SENT.clear()
        flit.OnRefresh(1); flit.OnUpdateBeatIndicator(1)
        self.assertEqual([o for (d, i, o) in flit.SENT if d == flit.DIR_EVENT], [], "fallback heartbeat only when OnIdle has been quiet for a second")
        flit._last_heartbeat -= 2.0
        flit.OnUpdateBeatIndicator(1)
        self.assertEqual([o["ev"] for (d, i, o) in flit.SENT if d == flit.DIR_EVENT], ["heartbeat"])
        flit.OnInit()
        self.assertEqual([o["ev"] for (d, i, o) in flit.SENT if d == flit.DIR_EVENT][-1], "hello")
        flit.OnProjectLoad(100)
        self.assertEqual(flit.SENT[-1][2]["ev"], "project_loaded")


if __name__ == "__main__":
    unittest.main()
