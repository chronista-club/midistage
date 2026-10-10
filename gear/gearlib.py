"""意味の層（`gear/<id>/gear.json`）を読む・揃える・対応を付ける純粋関数（docs/design/04）。

Blender に依存しない。`build.py` / `check.py` が Blender の中からここを import する。

- 機材 ID は inventory（crates/midistaged/src/inventory.rs）を正とする
- 部品名（`pad_1`、1 始まり）は触らない。`control`（`pad.0`、ControlEvent の 0 始まり index）を
  対応として書く。profile が読まない部品には付けない
- JSON は mm。座標は機材の中心が原点、x = 右 +、z = 手前 +
"""
import hashlib
import json
import os

SCHEMA_VERSION = 1

#: Ladyland 時代の資産 ID → inventory の profile ID
LEGACY_IDS = {"ncxse": "numa", "fgdp50": "fgdp"}

GEAR_IDS = ["roto", "lpd8", "nanokontrol", "keystage", "numa", "minilab", "fgdp", "xtouch"]

#: kind → ControlEvent の種類（既定の対応。section の並び順で index を振る）
KIND_TO_CONTROL = {"pad": "pad", "knob": "knob", "encoder": "knob", "fader": "fader", "button": "button"}


def canonical_id(gear_id):
    return LEGACY_IDS.get(gear_id, gear_id)


def normalize(spec):
    """旧 ID を inventory の ID に揃える。機種名は `model`、.blend の root 名は `blend_root` に残す"""
    out = dict(spec)
    old = spec["id"]
    new = canonical_id(old)
    out["id"] = new
    out.setdefault("model", spec.get("title", new))
    out.setdefault("blend_root", old)
    out["sections"] = [
        dict(s, id=s["id"].replace(f"{old}.", f"{new}.", 1)) for s in spec.get("sections", [])
    ]
    return out


# ---- control の対応 ----------------------------------------------------------

def _nanokontrol(spec):
    """crates/midistage-profiles device_input.rs `nanokontrol` と同じ番号。
    S = 0-7、M = 8-15、R = 16-23、TRACK ◀ ▶ = 24 / 25。transport は profile が読まない"""
    c = {}
    for n in range(1, 9):
        c[f"fader_{n}"] = f"fader.{n - 1}"
        c[f"knob_{n}"] = f"knob.{n - 1}"
        c[f"s_{n}"] = f"button.{n - 1}"
        c[f"m_{n}"] = f"button.{n + 7}"
        c[f"r_{n}"] = f"button.{n + 15}"
    c["track_prev"], c["track_next"] = "button.24", "button.25"
    return c


def _xtouch(spec):
    """device_input.rs `xtouch`: Fader 0-7 = ch1-8、8 = master。ボタン・エンコーダーは未定義"""
    c = {f"fader_{n}": f"fader.{n - 1}" for n in range(1, 9)}
    c["fader_master"] = "fader.8"
    return c


def _by_sections(spec):
    """既定: section の kind と並び順で index を振る（pads → pad.N、knobs → knob.N …）"""
    c = {}
    counters = {}
    for section in spec.get("sections", []):
        kind = {"pads": "pad", "knobs": "knob", "faders": "fader", "buttons": "button"}.get(section["kind"])
        if not kind:
            continue
        for name in section["parts"]:
            i = counters.get(kind, 0)
            c[name] = f"{kind}.{i}"
            counters[kind] = i + 1
    return c


RULES = {"nanokontrol": _nanokontrol, "xtouch": _xtouch, "lpd8": _by_sections}


def control_map(spec):
    """部品名 → control ID。profile に専用の規則があればそれ、無ければ section 順"""
    rule = RULES.get(spec["id"], _by_sections)
    return rule(spec)


def with_controls(spec):
    """各部品に `control` を書き込んだ写し（無い部品には付けない）"""
    c = control_map(spec)
    out = dict(spec)
    out["parts"] = [dict(p, control=c[p["name"]]) if p["name"] in c else dict(p) for p in spec["parts"]]
    for p in out["parts"]:
        if p["name"] not in c:
            p.pop("control", None)
    return out


# ---- 鍵盤の展開（Ladyland の gear_build.py から持ち越し） ------------------------

_BLACK = {1, 3, 6, 8, 10}
_BLACK_OFFSET = {1: -0.08, 3: 0.08, 6: -0.12, 8: 0.0, 10: 0.12}


def expand_keybed(spec):
    kb = spec.get("keybed")
    if not kb:
        return []
    ww, wl = kb["white_width"], kb["white_length"]
    bw, bl = kb["black_width"], kb["black_length"]
    h, base = kb.get("height", 12), kb.get("base", 6)
    parts = []
    white_index = -1
    for i in range(kb["count"]):
        note = kb["lowest"] + i
        semitone = note % 12
        if semitone not in _BLACK:
            white_index += 1
            x = kb["left"] + ww * (white_index + 0.5)
            parts.append({"name": f"key_{note}", "kind": "key_white",
                          "center": [x, kb["front"] - wl / 2], "size": [ww - 0.6, wl, h], "base": base})
        else:
            x = kb["left"] + ww * (white_index + 1) + ww * _BLACK_OFFSET[semitone]
            parts.append({"name": f"key_{note}", "kind": "key_black",
                          "center": [x, kb["front"] - wl + bl / 2], "size": [bw, bl, h + 6], "base": base})
    return parts


def expanded(spec):
    """アプリに配る形（鍵盤を key_<midi> に展開済み）"""
    out = dict(spec)
    out["parts"] = list(spec["parts"]) + expand_keybed(spec)
    return out


# ---- 読み書き ---------------------------------------------------------------

def load(root, gear_id):
    """`gear/<id>/gear.json` を読み、ID を揃え、control を付けて返す"""
    with open(os.path.join(root, gear_id, "gear.json")) as f:
        spec = json.load(f)
    return with_controls(normalize(spec))


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def manifest(dist, source_commit, gear_ids):
    """配布物の目録: schema / 生成元 commit / 機材ごとのファイルと sha256"""
    gear = {}
    for gid in gear_ids:
        files = {}
        for ext in ("usdz", "glb", "json"):
            path = os.path.join(dist, f"{gid}.{ext}")
            if os.path.exists(path):
                files[ext] = {"sha256": sha256(path), "bytes": os.path.getsize(path)}
        gear[gid] = files
    return {"schema": SCHEMA_VERSION, "source_commit": source_commit, "gear": gear}
