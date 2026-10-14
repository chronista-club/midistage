"""全高を仕様値に合わせる（docs/design/04 §7）。Blender の中で 1 機種ずつ回す。

    Blender --background gear/<id>/<id>.blend --python gear/polish_height.py -- [--save]

仮定: **仕様の全高 = 一番高い操作子の天面**。縦の内訳（筐体の厚み・段・ノブの高さ）は
出典で「推定」とされているので、その内訳を同じ比率 f = 仕様の全高 / 模型の全高 で伸ばす。

- 見た目の層: 各メッシュの頂点を機材の床（z = 0）基準で f 倍する（オブジェクトの
  位置・親子は触らない。共有メッシュは先に単独化する）。平面（x, y）は触らない
- 意味の層: gear.json の縦の数字を `gearlib.scale_heights` で同じ f 倍する
--save が無ければ測って報告するだけ。
"""
import json
import os
import sys

import bpy
from mathutils import Matrix, Vector

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gearlib  # noqa: E402


def descendants(root):
    out, stack = [], [root]
    while stack:
        o = stack.pop()
        out.append(o); stack.extend(o.children)
    return out


def top_mm(objs):
    return max((o.matrix_world @ Vector(c)).z for o in objs if o.type == "MESH" for c in o.bound_box) * 1000


def main():
    save = "--save" in sys.argv
    gid = os.path.basename(bpy.data.filepath).split(".")[0]
    path = os.path.join(HERE, gid, "gear.json")
    with open(path) as f:
        raw = json.load(f)
    spec = gearlib.normalize(raw)
    root = bpy.data.objects[spec.get("blend_root", gid)]
    objs = descendants(root)
    before = top_mm(objs)
    target = spec["size"][2]
    factor = target / before
    # 共有メッシュを単独化してから、ワールド座標の z を f 倍した位置へ頂点を戻す
    scale = Matrix.Diagonal((1.0, 1.0, factor, 1.0))
    for o in objs:
        if o.type != "MESH":
            continue
        if o.data.users > 1:
            o.data = o.data.copy()
        m = o.matrix_world.copy()
        o.data.transform(m.inverted() @ scale @ m)
        o.data.update()
    bpy.context.view_layer.update()
    after = top_mm(objs)
    print(f"POLISH {gid} spec={target} before={before:.2f} factor={factor:.4f} after={after:.2f}")
    if save:
        bpy.ops.wm.save_mainfile()
        with open(path, "w") as f:
            # 既存の書式（indent=1、末尾改行なし）に揃えて差分を縦の数字だけにする
            json.dump(gearlib.scale_heights(raw, factor), f, ensure_ascii=False, indent=1)
        print(f"SAVED {gid}")


main()
