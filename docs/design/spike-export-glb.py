import bpy, sys, os, json
out = sys.argv[sys.argv.index("--")+1]
gear_id = os.path.splitext(os.path.basename(bpy.data.filepath))[0]
# 機材コレクションのオブジェクトだけ選ぶ（確認用カメラ・ライトは除く）
bpy.ops.object.select_all(action='DESELECT')
names = []
for ob in bpy.data.objects:
    if ob.type in ('MESH', 'EMPTY'):
        ob.select_set(True); names.append(ob.name)
path = os.path.join(out, f"{gear_id}.glb")
bpy.ops.export_scene.gltf(filepath=path, export_format='GLB', use_selection=True,
    export_apply=True, export_yup=True, export_materials='EXPORT', export_image_format='AUTO')
json.dump({"gear": gear_id, "objects": sorted(names), "bytes": os.path.getsize(path)},
          open(os.path.join(out, f"{gear_id}.glb.json"), "w"), ensure_ascii=False, indent=1)
print("GLB_OK", path, os.path.getsize(path), len(names))
