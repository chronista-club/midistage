"""両層の契約を機械で守る（docs/design/04）。CI と、.blend を手で磨いたあとに回す。

    /Applications/Blender.app/Contents/MacOS/Blender --background --python gear/check.py -- [--only id,id]

検査:
  1. gear.json の部品（鍵盤展開込み）が全部 .blend にオブジェクトとして在る（名前の契約）
  2. root は原点、縮尺 1、回転 0
  3. root の外形（子孫の bbox）が gear.json の size（W × D × H mm）と ±2 mm で合う
     （フェーダーは可動域ぶんだけ奥行きが伸びうるので D は +travel まで許す）。
     **高さは警告止まり** — 生成模型の高さが仕様値より 3〜12 mm 低い機種が残っている
     （2026-10-11 移設時の実測。見た目の層を磨くときに合わせる）
失敗は 1 つでも exit 1（警告は exit に数えない）。
"""
import os
import sys

import bpy
from mathutils import Vector

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gearlib  # noqa: E402

TOL = 2.0  # mm


def descendants(root):
    out, stack = [], [root]
    while stack:
        o = stack.pop()
        out.append(o); stack.extend(o.children)
    return out


def bbox_mm(objs):
    lo = Vector((1e9, 1e9, 1e9)); hi = Vector((-1e9, -1e9, -1e9))
    for o in objs:
        if o.type != "MESH":
            continue
        for c in o.bound_box:
            w = o.matrix_world @ Vector(c)
            lo = Vector(map(min, lo, w)); hi = Vector(map(max, hi, w))
    return (hi - lo) * 1000  # m → mm。Blender: x 幅、y 奥行き、z 高さ


def check(gid):
    spec = gearlib.expanded(gearlib.load(HERE, gid))
    bpy.ops.wm.open_mainfile(filepath=os.path.join(HERE, gid, f"{gid}.blend"))
    root = bpy.data.objects.get(spec.get("blend_root", gid))
    problems = []
    if root is None:
        return [f"{gid}: root {spec.get('blend_root', gid)!r} が無い"]
    names = {o.name for o in descendants(root)}
    missing = [p["name"] for p in spec["parts"] if p["name"] not in names]
    if missing:
        problems.append(f"{gid}: .blend に無い部品 {len(missing)} 個: {missing[:8]}{' …' if len(missing) > 8 else ''}")
    if any(abs(v) > 1e-6 for v in root.location):
        problems.append(f"{gid}: root が原点に無い {tuple(root.location)}")
    if any(abs(v - 1) > 1e-6 for v in root.scale):
        problems.append(f"{gid}: root の縮尺が 1 でない {tuple(root.scale)}")
    if any(abs(v) > 1e-6 for v in root.rotation_euler):
        problems.append(f"{gid}: root が回転している {tuple(root.rotation_euler)}")
    w, d, h = spec["size"]
    travel = max([p.get("travel", 0) for p in spec["parts"]] + [0])
    bw, bd, bh = bbox_mm(descendants(root))
    if abs(bw - w) > TOL:
        problems.append(f"{gid}: 幅 {bw:.1f} ≠ {w}")
    if not (d - TOL <= bd <= d + travel + TOL):
        problems.append(f"{gid}: 奥行き {bd:.1f} ≠ {d}（+travel {travel}）")
    if abs(bh - h) > TOL:
        print(f"  WARN {gid}: 高さ {bh:.1f} ≠ {h}（仕様値。模型を磨くときに合わせる）")
    return problems


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    ids = argv[argv.index("--only") + 1].split(",") if "--only" in argv else gearlib.GEAR_IDS
    bad = []
    for gid in ids:
        p = check(gid)
        print(f"CHECK {'NG' if p else 'OK'} {gid}" + ("".join("\n  " + x for x in p)))
        bad += p
    if bad:
        sys.exit(1)


main()
