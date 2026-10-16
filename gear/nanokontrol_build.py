"""nanoKONTROL2 の模型を Blender で組む（2026-10-04。Ladyland の Gear/nanokontrol.py から 2026-10-14 に移した）。
書き出しは build.py、呼び出しは generate.py（`-- nanokontrol --output …`）。

アプリの 3D の机（`Desk3DView`）は
`~/Library/Application Support/ladyland/gear/nanokontrol.usdz` があればそれを読み、
**部品を名前で掴む**（`fader_1`〜`8` を音量の位置へ動かす、`m_1`〜`8` をミュートで赤く）。
寸法と配置は `Sources/Ladyland/Gear3D.swift` の `GearBlueprint.nanoKontrol2` と同じ
（外形は取扱説明書の 325 × 83 × 30 mm、配置は KORG 公式の真上の写真のトレース）。清書するときも部品名は保つこと。

使い方: Blender の Scripting タブで実行するか、Blender MCP の execute_blender_code に流す。
「nanoKONTROL2」コレクションだけを作り直し、他のオブジェクトと選択には触らない。
座標: アプリ (x, z) mm → Blender (x, -z)。書き出しは Y-up / 前 = -Z。
"""

import os

import bmesh
import bpy
from mathutils import Vector

MM = 0.001
COLL = "nanoKONTROL2"
TOP = 16.0  # 筐体の天面の高さ mm


def material(name, rgb, rough=0.5, metal=0.0):
    m = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    m.use_nodes = True
    # ⚠️ 節点名は UI の言語で訳される（日本語 UI では「プリンシプル BSDF」）— 種類で引く
    bsdf = next(n for n in m.node_tree.nodes if n.bl_idname == "ShaderNodeBsdfPrincipled")
    bsdf.inputs["Base Color"].default_value = (*rgb, 1)
    bsdf.inputs["Roughness"].default_value = rough
    bsdf.inputs["Metallic"].default_value = metal
    m.diffuse_color = (*rgb, 1)
    return m


def build():
    prev_sel = [o.name for o in bpy.context.selected_objects]
    prev_active = bpy.context.view_layer.objects.active

    if COLL in bpy.data.collections:
        old = bpy.data.collections[COLL]
        for o in list(old.objects):
            bpy.data.objects.remove(o, do_unlink=True)
        bpy.data.collections.remove(old)
    coll = bpy.data.collections.new(COLL)
    bpy.context.scene.collection.children.link(coll)

    mats = {
        "body": material("nk_body", (0.018, 0.018, 0.02), 0.45, 0.25),
        "groove": material("nk_groove", (0.004, 0.004, 0.004), 0.9),
        "cap": material("nk_cap", (0.03, 0.03, 0.032), 0.55),
        "knob": material("nk_knob", (0.05, 0.05, 0.055), 0.5),
        "button": material("nk_button", (0.09, 0.09, 0.095), 0.7),
        "white": material("nk_white", (0.9, 0.9, 0.88), 0.4),
    }

    def finish(ob, mat, parent, loc_mm, bevel_mm, segments=3):
        ob.data.materials.append(mat)
        coll.objects.link(ob)
        if parent:
            ob.parent = parent
        ob.location = Vector(loc_mm) * MM
        if bevel_mm > 0:
            b = ob.modifiers.new("bevel", "BEVEL")
            b.width = bevel_mm * MM
            b.segments = segments
        return ob

    def box(name, size_mm, loc_mm, mat, parent=None, bevel_mm=0.0):
        me = bpy.data.meshes.new(name)
        bm = bmesh.new()
        bmesh.ops.create_cube(bm, size=1.0)
        bmesh.ops.scale(bm, vec=Vector(size_mm) * MM, verts=bm.verts)
        bm.to_mesh(me)
        bm.free()
        return finish(bpy.data.objects.new(name, me), mat, parent, loc_mm, bevel_mm)

    def cylinder(name, radius_mm, depth_mm, loc_mm, mat, parent=None):
        me = bpy.data.meshes.new(name)
        bm = bmesh.new()
        bmesh.ops.create_cone(
            bm, cap_ends=True, segments=32, radius1=radius_mm * MM,
            radius2=radius_mm * MM * 0.92, depth=depth_mm * MM)
        bm.to_mesh(me)
        bm.free()
        for p in me.polygons:
            p.use_smooth = True
        return finish(bpy.data.objects.new(name, me), mat, parent, loc_mm, 0.6, 2)

    root = bpy.data.objects.new("nanokontrol", None)
    root.empty_display_size = 0.05
    coll.objects.link(root)

    box("body", (325, 83, TOP), (0, 0, TOP / 2), mats["body"], root, bevel_mm=2.5)

    # 配置は KORG 公式の真上からの写真を画素で測ったトレース（Gear3D.swift と同じ値）。
    # 横 3 行 = S/M/R の行、左側の縦 5 列
    row = [-4.3, 10.7, 25.9]
    col = [-144.3, -128.6, -113.0, -97.2, -81.5]
    pill, square = (10.8, 4.6), (11.4, 10.9)
    transport = [
        ("track_prev", col[0], row[0], pill), ("track_next", col[1], row[0], pill),
        ("cycle", col[0], row[1], pill), ("marker_set", col[2], row[1], pill),
        ("marker_prev", col[3], row[1], pill), ("marker_next", col[4], row[1], pill),
        ("rew", col[0], row[2], square), ("ff", col[1], row[2], square), ("stop", col[2], row[2], square),
        ("play", col[3], row[2], square), ("rec", col[4], row[2], square),
    ]
    for name, x, z, (w, d) in transport:
        bevel = 1.6 if d < 6 else 1.0  # 細長い楕円は角を大きく丸める
        box(name, (w, d, 3), (x, -z, TOP + 1.5), mats["button"], root, bevel_mm=bevel)

    # 電源の LED（飾り — アプリは掴まない）
    box("led", (6.3, 2.3, 0.4), (-150.1, 31.8, TOP + 0.2), mats["white"], root, bevel_mm=0.9)

    strip = [-61.2, -33.6, -6.3, 21.5, 48.8, 76.4, 103.9, 131.3]
    for i, sx in enumerate(strip):
        n = i + 1
        knob = cylinder(f"knob_{n}", 6.0, 13.0, (sx + 13.5, 27.3, TOP + 6.5), mats["knob"], root)
        box(f"knob_{n}_mark", (1.0, 4.5, 0.3), (0, 2.6, 6.6), mats["white"], knob)
        for name, z in zip(("s", "m", "r"), row):
            box(f"{name}_{n}", (8.8, 8.9, 3), (sx, -z, TOP + 1.5), mats["button"], root, bevel_mm=1.0)
        fx = sx + 12.95
        box(f"fader_slot_{n}", (2.6, 38.9, 0.6), (fx, -row[1], TOP + 0.3), mats["groove"], root)
        cap = box(f"fader_{n}", (8.5, 22, 9), (fx, -row[1], TOP + 4.5), mats["cap"], root, bevel_mm=1.2)
        for k in range(5):  # 滑り止めの溝（写真の横筋）
            box(f"fader_{n}_rib_{k}", (8.0, 0.6, 0.3), (0, -6 + 3 * k, 4.6), mats["groove"], cap)
        box(f"fader_{n}_line", (8.0, 1.0, 0.3), (0, 0, 4.65), mats["white"], cap)

    for o in bpy.context.selected_objects:
        o.select_set(False)
    for name in prev_sel:
        if name in bpy.data.objects:
            bpy.data.objects[name].select_set(True)
    bpy.context.view_layer.objects.active = prev_active
    return coll
