"""名前の契約を直す（1 回きりの移設作業。Blender の中で回す）。

8 台が 1 つのシーンに同居していた頃、同名の部品に Blender が `.001` 等の接尾辞を付けた
（`knob_1.003` など）。機材ごとに分けた今は衝突しないので、接尾辞を外して gear.json の
部品名に戻す。外した名前が既に別オブジェクトに使われている場合は触らず報告する。

    Blender --background gear/<id>/<id>.blend --python gear/fix_names.py -- [--save]
"""
import os
import re
import sys

import bpy

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gearlib  # noqa: E402

SUFFIX = re.compile(r"^(.*)\.(\d{3})$")


def main():
    save = "--save" in sys.argv
    gid = os.path.basename(bpy.data.filepath).split(".")[0]
    spec = gearlib.expanded(gearlib.load(HERE, gid))
    want = {p["name"] for p in spec["parts"]}
    taken = {o.name for o in bpy.data.objects}
    renamed, blocked = [], []
    for o in list(bpy.data.objects):
        m = SUFFIX.match(o.name)
        if not m:
            continue
        base = m.group(1)
        if base in taken:
            blocked.append((o.name, base))
            continue
        o.name = base
        taken.add(base)
        renamed.append((m.group(0), base))
    still = sorted(want - {o.name for o in bpy.data.objects})
    print(f"FIX {gid} renamed={len(renamed)} blocked={len(blocked)} still_missing={still[:6]}")
    for a, b in blocked[:10]:
        print(f"  blocked {a} → {b}（{b} は使用中）")
    if save:
        bpy.ops.wm.save_mainfile()


main()
