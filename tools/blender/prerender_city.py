"""Headless city pre-render from the engine's own layout export.

Composites the REAL level layout (game-side `RV3D_EXPORT_CITY=<path>` JSON:
sun + WorldMarker boxes + GLB prop placements) into a clean Blender scene and
renders camera views to PNG. This is an independent visual ground truth for
modelling / lighting / bake iteration: it never goes through the game's raster
pipeline, so defects it shows are asset/layout defects, not shader artifacts.

Mouse safety: headless only (`--background`), never the GUI (AGENTS.md 铁律 D).

Run:
  blender.exe --background --python tools/blender/prerender_city.py -- \
      <city.json> <out_prefix> [cam ...] [--markers] [--engine EEVEE|CYCLES]

  cam strings use the game's RV3D_CAM syntax: "x,y,z:yaw,pitch" (game coords,
  degrees). Default camera = the boulevard probe view "0,1.7,30:180,2".
  --markers additionally renders marker boxes as tinted cubes (layout QA).

Coordinate mapping (game right-handed Y-up meters -> Blender Z-up):
  (x, y, z)_game -> (x, -z, y)_blender ; prop yaw about game +Y -> Blender +Z.
"""
import sys
import os
import json
import math
import bpy
from mathutils import Vector

# ------------------------------------------------------------------ argv
argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
if len(argv) < 2:
    raise SystemExit("usage: blender --background --python prerender_city.py -- "
                     "<city.json> <out_prefix> [cam ...] [--markers] [--engine E]")
json_path = argv[0]
out_prefix = argv[1]
rest = argv[2:]
show_markers = "--markers" in rest
rest = [r for r in rest if r != "--markers"]
engine_req = "EEVEE"
if "--engine" in rest:
    i = rest.index("--engine")
    engine_req = rest[i + 1]
    rest = rest[:i]
cams = rest or ["0,1.7,30:180,2"]

with open(json_path, "r", encoding="utf-8") as f:
    city = json.load(f)

def g2b(p):
    """game (x, y_up, z) -> blender (x, -z, y)"""
    return Vector((p[0], -p[2], p[1]))

# ------------------------------------------------------------------ clean scene
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene

# ------------------------------------------------------------------ world (game sky clear color, linear)
world = bpy.data.worlds.new("cityWorld")
scene.world = world
world.use_nodes = True
bg = world.node_tree.nodes.get("Background")
if bg:
    bg.inputs[0].default_value = (0.24, 0.36, 0.60, 1.0)
    bg.inputs[1].default_value = 1.0

# ------------------------------------------------------------------ sun (same source as game light_uniform)
sun_c = city["sun"]
d = g2b(sun_c["dir"])          # surface -> sun direction, blender space
d.normalize()
sun_data = bpy.data.lights.new("citySun", type='SUN')
# game intensity 1.35 sits in the engine's tone curve; Blender sun needs its own
# exposure scale -- 4.0 approximates mid-day look under Filmic at 1spp-ish QA use.
sun_data.energy = float(sun_c["intensity"]) * 3.0
sun_data.color = (sun_c["color"][0], sun_c["color"][1], sun_c["color"][2])
sun_data.angle = math.radians(3.0)
sun = bpy.data.objects.new("citySun", sun_data)
sun.rotation_euler = d.to_track_quat('Z', 'Y').to_euler()
scene.collection.objects.link(sun)

# ------------------------------------------------------------------ ground plane
# sits at game ground level (UNDER_GROUND = -0.05): an eye-height camera at
# y=1.7 must be ABOVE this plane, or the plane's backface fills the frame
bpy.ops.mesh.primitive_plane_add(size=900.0, location=(0, 0, -0.05))
ground = bpy.context.active_object
ground.name = "cityGround"
try:
    ground.visible_face_culling = True   # Blender 5.x: object-level viewport/render culling
except AttributeError:
    pass
gmat = bpy.data.materials.new("cityGround")
gmat.use_nodes = True
gb = gmat.node_tree.nodes.get("Principled BSDF")
if gb:
    gb.inputs["Base Color"].default_value = (0.115, 0.120, 0.128, 1.0)  # asphalt, PT box 0 同源
    gb.inputs["Roughness"].default_value = 0.9
ground.data.materials.append(gmat)

# ------------------------------------------------------------------ prop library: import each GLB once
props_dir = "assets/props"
if not os.path.isdir(props_dir):
    props_dir = os.path.join(os.path.dirname(os.path.dirname(
        os.path.dirname(os.path.abspath(__file__)))), "assets", "props")
lib = {}   # name -> single joined template object (hidden)
def get_template(name):
    if name in lib:
        return lib[name]
    fp = os.path.join(props_dir, name + ".glb")
    if not os.path.isfile(fp):
        lib[name] = None
        return None
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=fp)
    added = [o for o in bpy.data.objects if o not in before]
    meshes = [o for o in added if o.type == 'MESH']
    others = [o for o in added if o.type != 'MESH']
    if not meshes:
        for o in added:
            bpy.data.objects.remove(o, do_unlink=True)
        lib[name] = None
        return None
    bpy.context.view_layer.update()
    # detach from imported node empties keeping world placement, then drop empties
    for o in meshes:
        mw = o.matrix_world.copy()
        o.parent = None
        o.matrix_world = mw
    for o in others:
        bpy.data.objects.remove(o, do_unlink=True)
    # join all meshes into one template so per-placement copies are single objects
    bpy.ops.object.select_all(action='DESELECT')
    for o in meshes:
        o.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    if len(meshes) > 1:
        bpy.ops.object.join()
    t = bpy.context.view_layer.objects.active
    t.name = "lib_" + name
    t.location = (0.0, 0.0, 0.0)
    t.hide_render = True
    t.hide_viewport = True
    # the game's raster culls backfaces on EVERYTHING; a camera inside a prop must
    # see through it here too, or audit frames go solid (plaza cam sits in a trunk)
    for slot in t.material_slots:
        mat = slot.material
        if mat is None:
            continue
        try:
            mat.use_backface_culling = True
        except AttributeError:
            mat.backface_culling = 'BACK'
    lib[name] = t
    return t

# hide templates from render after linking duplicates
placed = 0
missing = {}
for p in city["props"]:
    t = get_template(p["mesh"])
    if t is None:
        missing[p["mesh"]] = missing.get(p["mesh"], 0) + 1
        continue
    dup = t.copy()           # linked duplicate: shares mesh data
    dup.name = "prop_%s_%d" % (p["mesh"], placed)
    scene.collection.objects.link(dup)
    dup.hide_render = False
    dup.hide_viewport = False
    dup.location = g2b((p["x"], p["y"], p["z"]))
    dup.rotation_euler = (0.0, 0.0, float(p["yaw"]))
    s = float(p["scale"])
    dup.scale = (s, s, s)
    placed += 1
for name, n in sorted(missing.items()):
    print("PRERENDER WARN missing mesh %s x%d" % (name, n))

# ------------------------------------------------------------------ marker boxes (optional)
mk_done = 0
if show_markers:
    mat_cache = {}
    for m in city["markers"]:
        key = tuple(round(c, 2) for c in m["tint"])
        if key not in mat_cache:
            mt = bpy.data.materials.new("mk_%g_%g_%g" % key)
            mt.use_nodes = True
            try:
                mt.use_backface_culling = True   # camera inside a box sees through it
            except AttributeError:
                mt.backface_culling = 'BACK'
            mb = mt.node_tree.nodes.get("Principled BSDF")
            if mb:
                mb.inputs["Base Color"].default_value = (key[0], key[1], key[2], 1.0)
                mb.inputs["Roughness"].default_value = 0.85
            mat_cache[key] = mt
        bpy.ops.mesh.primitive_cube_add(size=2.0)
        c = bpy.context.active_object
        c.name = "marker_%d" % mk_done
        c.location = g2b((m["x"], m["y"], m["z"]))
        c.scale = (m["hw"], m["hd"], m["hh"])   # game (w,d,h) -> blender (x,y,z)
        c.data.materials.append(mat_cache[key])
        mk_done += 1

# templates are hidden per-object (hide_render/viewport); copies un-hide explicitly

# ------------------------------------------------------------------ render setup
def pick_engine(req):
    order = ("BLENDER_EEVEE_NEXT", "BLENDER_EEVEE") if req == "EEVEE" else ("CYCLES",)
    for name in order:
        try:
            scene.render.engine = name
            return name
        except TypeError:
            continue
    return scene.render.engine

engine = pick_engine(engine_req)
if engine == "CYCLES":
    scene.cycles.samples = 32
else:
    try:
        scene.eevee.taa_render_samples = 16
    except Exception:
        pass

scene.render.resolution_x = 1280
scene.render.resolution_y = 800
scene.render.resolution_percentage = 100
scene.render.image_settings.file_format = 'PNG'
try:
    scene.view_settings.view_transform = 'Filmic'
except AttributeError:
    pass

# ------------------------------------------------------------------ cameras (game RV3D_CAM syntax)
def game_cam_to_blender(spec):
    if not all(ch in "0123456789.,:-" for ch in spec.replace(":", "")):
        raise ValueError("cam spec must be ASCII digits/punct, got %r "
                         "(non-ASCII minus/comma from shell encoding is a known trap)" % spec)
    pos_s, rot_s = spec.split(":")
    px, py, pz = (float(v) for v in pos_s.split(","))
    yaw, pitch = (float(v) for v in rot_s.split(","))
    yr, pr = math.radians(yaw), math.radians(pitch)
    # game convention: POSITIVE pitch looks DOWN (RV3D_CAM 第 14 轮教训的同一符号)
    fwd = Vector((-math.sin(yr) * math.cos(pr), -math.sin(pr), -math.cos(yr) * math.cos(pr)))
    loc = g2b((px, py, pz))
    dirb = g2b(fwd)
    dirb.normalize()
    quat = (-dirb).to_track_quat('Z', 'Y')   # camera looks along -Z
    return loc, quat.to_euler()

for i, spec in enumerate(cams):
    # audit hygiene: report whatever swallows the camera (game culls box/prop
    # interiors; EEVEE may not) so a flat frame is explained, not guessed at
    gp = spec.split(":")[0]
    gx, gy, gz = (float(v) for v in gp.split(","))
    inside = [m["kind"] for m in city["markers"]
              if abs(m["x"] - gx) <= m["hw"] and abs(m["z"] - gz) <= m["hd"]
              and m["y"] - m["hh"] <= gy <= m["y"] + m["hh"]]
    near_props = [p["mesh"] for p in city["props"]
                  if ((p["x"] - gx) ** 2 + (p["z"] - gz) ** 2) ** 0.5 < 1.5
                  and p["y"] <= gy <= p["y"] + 8.0]
    if inside or near_props:
        print("PRERENDER WARN cam%d %s inside markers=%s near props=%s"
              % (i, spec, inside, near_props))
    cam_data = bpy.data.cameras.new("cam%d" % i)
    cam_data.lens = 35.0   # ~60 deg hfov-ish
    cam = bpy.data.objects.new("cam%d" % i, cam_data)
    scene.collection.objects.link(cam)
    cam.location, cam.rotation_euler = game_cam_to_blender(spec)
    scene.camera = cam
    out = "%s_%d.png" % (out_prefix, i)
    if not os.path.isabs(out):
        repo_root = os.path.dirname(os.path.dirname(
            os.path.dirname(os.path.abspath(__file__))))
        out = os.path.join(repo_root, out)
    scene.render.filepath = out
    bpy.ops.render.render(write_still=True)
    print("PRERENDER wrote %s (cam %s)" % (out, spec))

print("PRERENDER engine=%s props=%d markers=%d meshes_imported=%d"
      % (engine, placed, mk_done, sum(1 for v in lib.values() if v is not None)))
