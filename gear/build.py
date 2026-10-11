"""見た目の層（`gear/<id>/<id>.blend`）から配布物を書き出す（docs/design/04）。

    /Applications/Blender.app/Contents/MacOS/Blender --background --python gear/build.py -- \
        [--output DIR] [--only id,id] [--commit SHA]

出力（既定 `~/Library/Application Support/midistage/gear/`）:
  <id>.usdz   RealityKit 向け（アプリ A）
  <id>.glb    Web / Three.js 向け（アプリ B）
  <id>.json   意味の層（ID を揃え、control を付け、鍵盤を展開済み）
  manifest.json  schema / 生成元 commit / sha256

git に入れるのは生成元だけ。ここで .blend は保存しない。
"""
import json
import os
import subprocess
import sys

import bpy

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gearlib  # noqa: E402

DEFAULT_OUT = os.path.expanduser("~/Library/Application Support/midistage/gear")


def args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    out, only, commit = DEFAULT_OUT, None, None
    i = 0
    while i < len(argv):
        if argv[i] == "--output":
            out = argv[i + 1]; i += 2
        elif argv[i] == "--only":
            only = argv[i + 1].split(","); i += 2
        elif argv[i] == "--commit":
            commit = argv[i + 1]; i += 2
        else:
            raise SystemExit(f"unknown arg {argv[i]}")
    return out, only, commit


def git_commit():
    try:
        return subprocess.check_output(["git", "-C", HERE, "rev-parse", "--short", "HEAD"], text=True).strip()
    except Exception:
        return "unknown"


def gear_objects(root_name):
    """機材の root とその子孫だけ（確認用カメラ・ライトは除く）"""
    root = bpy.data.objects.get(root_name)
    if root is None:
        raise SystemExit(f"root object {root_name!r} not found")
    objs = [root]
    stack = [root]
    while stack:
        for child in stack.pop().children:
            objs.append(child); stack.append(child)
    return objs


def select_only(objs):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]


def export(spec, out):
    gid = spec["id"]
    root_name = spec.get("blend_root", gid)
    bpy.ops.wm.open_mainfile(filepath=os.path.join(HERE, gid, f"{gid}.blend"))
    objs = gear_objects(root_name)
    # 配布物の root 名は機材 ID（アプリは ID で掴む）。保存はしないので .blend は変わらない
    objs[0].name = gid
    select_only(objs)
    usdz = os.path.join(out, f"{gid}.usdz")
    bpy.ops.wm.usd_export(
        filepath=usdz, selected_objects_only=True, export_materials=True,
        generate_preview_surface=True, export_textures_mode="NEW", overwrite_textures=True,
        export_animation=False, convert_orientation=True,
        export_global_forward_selection="NEGATIVE_Z", export_global_up_selection="Y",
        evaluation_mode="RENDER", root_prim_path="/root")
    glb = os.path.join(out, f"{gid}.glb")
    bpy.ops.export_scene.gltf(
        filepath=glb, export_format="GLB", use_selection=True, export_apply=True,
        export_yup=True, export_materials="EXPORT", export_image_format="AUTO")
    with open(os.path.join(out, f"{gid}.json"), "w") as f:
        json.dump(gearlib.expanded(spec), f, ensure_ascii=False, indent=1)
    print(f"BUILD_OK {gid} usdz={os.path.getsize(usdz)} glb={os.path.getsize(glb)} parts={len(spec['parts'])}")


def main():
    out, only, commit = args()
    os.makedirs(out, exist_ok=True)
    ids = only or gearlib.GEAR_IDS
    for gid in ids:
        export(gearlib.load(HERE, gid), out)
    m = gearlib.manifest(out, commit or git_commit(), ids)
    with open(os.path.join(out, "manifest.json"), "w") as f:
        json.dump(m, f, ensure_ascii=False, indent=1)
    print(f"MANIFEST_OK {len(ids)} gear → {out}")


main()
