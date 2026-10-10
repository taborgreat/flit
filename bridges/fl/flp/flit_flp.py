#!/usr/bin/env python3
"""Read an FL Studio .flp and print a host-neutral JSON snapshot.

Uses PyFLP (GPL-3). It runs as a separate process so the GPL code is never
linked into FLit; the Rust side (bridges/fl/src/flp.rs) spawns it.

    python -I flit_flp.py path/to/song.flp            # JSON snapshot
    python -I flit_flp.py --diff a.flp b.flp          # what changed

Known limits (see ../RESEARCH.md §5): PyFLP is unmaintained, needs the
Python 3.12 monkeypatch below, and playlist items of FL 21+/2025 files are
unreliable. Channels, patterns, notes and mixer inserts parse.
VERIFY ON WINDOWS: run on a real FL 2025/2026 .flp from the user.
"""
import json
import sys
import warnings


def _patch_pyflp():
    """PyFLP #183: EventEnum base has no members on Python 3.12."""
    import pyflp._events as _ev
    meta = type(_ev.EventEnum)
    orig = meta.__call__

    def patched(cls, value, *args, **kwargs):
        if cls is _ev.EventEnum and not args and not kwargs:
            member = cls._missing_(value)
            if member is None:
                raise ValueError("%r is not a valid %s" % (value, cls.__name__))
            return member
        return orig(cls, value, *args, **kwargs)

    if getattr(meta, "_flit_patched", False):
        return
    meta.__call__ = patched
    meta._flit_patched = True


def key_to_midi(key):
    """'C5' -> 60 (PyFLP names C5 = middle C, like FL)."""
    if isinstance(key, int):
        return key
    names = {"C": 0, "C#": 1, "D": 2, "D#": 3, "E": 4, "F": 5, "F#": 6, "G": 7, "G#": 8, "A": 9, "A#": 10, "B": 11}
    k = str(key)
    i = 1
    while i < len(k) and not (k[i].isdigit() or k[i] == "-"):
        i += 1
    return names.get(k[:i], 0) + (int(k[i:]) + 0) * 12


def snapshot(path):
    import pyflp
    _patch_pyflp()
    warns = []
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always")
        p = pyflp.parse(path)
        warns = [str(x.message) for x in w]
    channels = []
    for c in p.channels:
        insert = None
        try:
            insert = int(c.insert) if c.insert is not None else None
        except Exception:
            insert = None
        plugin = None
        try:
            plugin = getattr(getattr(c, "plugin", None), "name", None) or (type(c.plugin).__name__ if getattr(c, "plugin", None) is not None else None)
        except Exception:
            plugin = None
        channels.append({"iid": int(c.iid), "name": c.name, "kind": type(c).__name__, "insert": insert, "plugin": plugin})
    patterns = []
    for pt in p.patterns:
        notes = []
        for n in pt.notes:
            notes.append({"position": int(n.position or 0), "length": int(n.length or 0), "key": key_to_midi(n.key), "velocity": int(n.velocity or 0), "channel": int(n.rack_channel or 0)})
        patterns.append({"iid": int(pt.iid), "name": pt.name, "notes": notes})
    inserts = []
    for ins in p.mixer:
        slots = []
        for s in ins:
            try:
                plugin = getattr(s.plugin, "name", None) or (type(s.plugin).__name__ if s.plugin is not None else None)
            except Exception:
                plugin = None
            if s.name is None and plugin is None:
                continue
            slots.append({"index": int(s.index), "name": s.name, "plugin": plugin})
        if ins.name is None and not slots and ins.iid > 0:
            continue
        inserts.append({"iid": int(ins.iid), "name": ins.name, "volume": int(ins.volume or 0), "slots": slots})
    return {
        "version": str(p.version),
        "ppq": int(p.ppq),
        "tempo": float(p.tempo),
        "channels": channels,
        "patterns": patterns,
        "inserts": inserts,
        "files": [],
        "warnings": warns,
    }


def diff(a, b):
    """Human-readable differences between two snapshots (what a save changed)."""
    out = []
    if a["tempo"] != b["tempo"]:
        out.append("tempo %s -> %s" % (a["tempo"], b["tempo"]))
    pa = {p["iid"]: p for p in a["patterns"]}
    for p in b["patterns"]:
        old = pa.get(p["iid"])
        if old is None:
            out.append("pattern %s added (%d notes)" % (p["name"] or p["iid"], len(p["notes"])))
        elif old["notes"] != p["notes"]:
            out.append("pattern %s notes changed (%d -> %d)" % (p["name"] or p["iid"], len(old["notes"]), len(p["notes"])))
    ia = {i["iid"]: i for i in a["inserts"]}
    for i in b["inserts"]:
        old = ia.get(i["iid"])
        if old is None:
            out.append("insert %s added" % (i["name"] or i["iid"]))
        else:
            if old["volume"] != i["volume"]:
                out.append("insert %s volume %s -> %s" % (i["name"] or i["iid"], old["volume"], i["volume"]))
            if old["slots"] != i["slots"]:
                out.append("insert %s plugins changed" % (i["name"] or i["iid"]))
    return out


def main(argv):
    if len(argv) >= 4 and argv[1] == "--diff":
        a, b = snapshot(argv[2]), snapshot(argv[3])
        for line in diff(a, b):
            print(line)
        return 0
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    print(json.dumps(snapshot(argv[1])))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
