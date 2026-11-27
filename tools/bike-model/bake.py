import bpy, math, mathutils, sys
out = sys.argv[sys.argv.index("--")+1]
D = bpy.data
# 1. Studio setup out.
for name in ["Camera","Light","Empty.001","Plane","Circle.007","Cylinder.022"] + [o.name for o in D.objects if o.name.startswith("alights")]:
    ob = D.objects.get(name)
    if ob: D.objects.remove(ob, do_unlink=True)
# 1b. No brake or gear cables (bevelled curves): out of the levers, along
# the frame and to the rear derailleur.
for name in ["Cube.010","Cube.019","Cube.020","Cube.021"]:
    ob = D.objects.get(name)
    if ob: D.objects.remove(ob, do_unlink=True)
# 2. Lighter shapes, then every part fixed in its final shape.
for o in D.objects:
    subs=[m for m in o.modifiers if m.type=='SUBSURF']
    for m in subs[1:]: o.modifiers.remove(m)
    for m in subs[:1]: m.levels=1; m.render_levels=1
chain=D.objects.get("Circle.003")
if chain:
    # Each link as a plain block of the same size: from the saddle the
    # chain reads as a band.
    import bmesh
    me=chain.data
    xs=[v.co.x for v in me.vertices]; ys=[v.co.y for v in me.vertices]; zs=[v.co.z for v in me.vertices]
    lo=(min(xs),min(ys),min(zs)); hi=(max(xs),max(ys),max(zs))
    bm=bmesh.new(); bmesh.ops.create_cube(bm,size=1.0)
    for v in bm.verts:
        v.co.x=lo[0]+(v.co.x+0.5)*(hi[0]-lo[0]); v.co.y=lo[1]+(v.co.y+0.5)*(hi[1]-lo[1]); v.co.z=lo[2]+(v.co.z+0.5)*(hi[2]-lo[2])
    mats=list(me.materials)
    bm.to_mesh(me); bm.free()
    me.materials.clear()
    for m in mats: me.materials.append(m)
bpy.ops.object.select_all(action='DESELECT')
keep_path="Circle.005"
todo=[o for o in D.objects if o.type in ('MESH','CURVE') and o.name!=keep_path]
for o in todo: o.select_set(True)
bpy.context.view_layer.objects.active=todo[0]
bpy.ops.object.convert(target='MESH')
D.objects.remove(D.objects[keep_path], do_unlink=True)
_c=[]
for o in D.objects:
    if o.type=='MESH':
        o.data.calc_loop_triangles(); _c.append((len(o.data.loop_triangles),o.name))
print("HEAVY", sorted(_c,reverse=True)[:8])
# 2b. Every part's faces turned outward (some were modelled inside out,
# which a glossy surface shows as a full reflection of the sky).
import bmesh
# First the parts' rotation and scale (some mirrored) into their shapes,
# so "outward" means outward in the world.
bpy.ops.object.select_all(action='DESELECT')
for o in D.objects:
    if o.type=='MESH': o.select_set(True)
bpy.context.view_layer.objects.active=[o for o in D.objects if o.type=='MESH'][0]
bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
for o in D.objects:
    if o.type=='MESH':
        bm=bmesh.new(); bm.from_mesh(o.data)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        bm.to_mesh(o.data); bm.free(); o.data.update()
# 3. Simple materials that glTF carries.
def principled(name, rgb, metallic, rough):
    m=D.materials[name]; m.use_nodes=True; nt=m.node_tree
    for n in list(nt.nodes): nt.nodes.remove(n)
    b=nt.nodes.new('ShaderNodeBsdfPrincipled'); o=nt.nodes.new('ShaderNodeOutputMaterial')
    b.inputs['Base Color'].default_value=(*rgb,1); b.inputs['Metallic'].default_value=metallic; b.inputs['Roughness'].default_value=rough
    nt.links.new(b.outputs[0], o.inputs[0]); m.diffuse_color=(*rgb,1)
PAINT=(0.62,0.015,0.02)
looks={"Material":(PAINT,0.0,0.25),"Material.001":((0.02,0.02,0.02),0.0,0.45),"Material.004":((0.02,0.02,0.02),0.0,0.45),
 "Material.010":((0.02,0.02,0.02),0.0,0.45),"Material.002":((0.07,0.07,0.08),0.4,0.35),"Material.006":((0.5,0.5,0.52),1.0,0.22),
 "Material.008":((0.025,0.025,0.025),0.0,0.85),"Material.009":((0.42,0.42,0.43),1.0,0.35),"Material.003":((0.8,0.8,0.8),0.0,0.5),
 "Material.005":((0.8,0.8,0.8),0.0,0.5),"Material.007":((0.8,0.8,0.8),0.0,0.5),"Material.011":((0.09,0.09,0.09),0.0,0.5),"Material.012":((0.4,0.4,0.4),0.0,0.5)}
for k,(c,m,r) in looks.items():
    if k in D.materials: principled(k,c,m,r)
# Parts with no material: dark grey.
dark=D.materials.new("Untinted"); principled("Untinted",(0.05,0.05,0.05),0.3,0.5)
for o in D.objects:
    if o.type=='MESH' and len(o.data.materials)==0: o.data.materials.append(dark)
# 4b. Lighter where it does not show: bars, hoods, calipers; rims less
# smooth. And the saddle lowered on its seatpost to meet the rider.
for name, ratio in (("Cylinder.038",0.3),("Cylinder.037",0.25),("Cylinder.084",0.25),("Cylinder.041",0.35),("Cube.001",0.5),("Cube.005",0.5),("Cube.010",0.4),("Circle.002",0.5),("Circle.006",0.5),("Cylinder.039",0.6),("Cylinder.001",0.25),("Cube.011",0.4),("Cube.012",0.4),("Cube.022",0.35),("Cube.023",0.35),("Cylinder.045",0.5),("Circle",0.7),("Circle.004",0.7),("Cube",0.6)):
    ob=D.objects.get(name)
    if ob and ob.type=='MESH':
        m=ob.modifiers.new("Lighter","DECIMATE"); m.ratio=ratio
        bpy.ops.object.select_all(action='DESELECT'); ob.select_set(True); bpy.context.view_layer.objects.active=ob
        bpy.ops.object.convert(target='MESH')
K=float(sys.argv[sys.argv.index("--")+2])
BB0=mathutils.Vector((0.0,-0.19,-0.54))
# The saddle's top 0.657 above the bottom bracket, where the rider's sit
# bones rest on its wide back (the body's underside, measured in its seated
# pose).
SEAT_HEIGHT=0.657
drop=(0.985-BB0.z) - SEAT_HEIGHT/K    # how far the saddle top must come down
seat_dir=mathutils.Vector((0.0,-0.43,1.525)).normalized()
for name in ("Cube.003","Cube.004"):  # seatpost and saddle
    ob=D.objects.get(name)
    if ob: ob.location -= seat_dir*(drop/seat_dir.z)
print("SEAT drop units", round(drop,3))
# 4c. Round wheels, turning true: the tyre and rim (modelled 2.5% oval)
# made round, and every part of each wheel centred on it.
def box_centre(o):
    pts=[o.matrix_world@v.co for v in o.data.vertices]
    return mathutils.Vector(((min(p.x for p in pts)+max(p.x for p in pts))/2,
                             (min(p.y for p in pts)+max(p.y for p in pts))/2,
                             (min(p.z for p in pts)+max(p.z for p in pts))/2)), pts
WHEEL_PARTS={"Circle":["Circle.002","Cylinder.023","Cylinder.037","Cylinder.038","Cylinder.042","Cylinder.046","Cube.009"],
             "Circle.004":["Circle.006","Cylinder.075","Cylinder.082","Cylinder.083","Cylinder.084"]}
HUBS={}
for rim_name, others in WHEEL_PARTS.items():
    rim=D.objects[rim_name]
    c,pts=box_centre(rim)
    h=max(p.y for p in pts)-min(p.y for p in pts); v=max(p.z for p in pts)-min(p.z for p in pts)
    inv=rim.matrix_world.inverted()
    for vert in rim.data.vertices:
        w=rim.matrix_world@vert.co
        w.y=c.y+(w.y-c.y)*(v/h)        # stretch the short way to match
        vert.co=inv@w
    rim.data.update()
    for n in others:
        o=D.objects.get(n)
        if not o: continue
        oc,_=box_centre(o)
        o.location += mathutils.Vector((0.0, c.y-oc.y, c.z-oc.z))
    HUBS[rim_name]=(0.0,c.y,c.z)
print("HUBS", {k:tuple(round(x,4) for x in v) for k,v in HUBS.items()})
# 5. Groups, each about its own centre.
groups={
 "rear_wheel":(["Circle","Circle.002","Cylinder.023","Cylinder.037","Cylinder.038","Cylinder.042","Cylinder.046","Cube.009","Cylinder.088"],HUBS["Circle"]),
 "front_wheel":(["Circle.004","Circle.006","Cylinder.075","Cylinder.082","Cylinder.083","Cylinder.084","Cylinder.087"],HUBS["Circle.004"]),
 "crankset":(["Cylinder.039","Cylinder.040","Cube.006","Cube.015"],(0,-0.19,-0.54)),
 "pedal_right":(["Cube.013","Cylinder.044"],None),
 "pedal_left":(["Cube.016","Cylinder.085"],None),
}
def join(names, newname, pivot):
    bpy.ops.object.select_all(action='DESELECT')
    obs=[D.objects[n] for n in names if n in D.objects]
    for o in obs: o.select_set(True)
    bpy.context.view_layer.objects.active=obs[0]
    if len(obs)>1: bpy.ops.object.join()
    ob=bpy.context.view_layer.objects.active; ob.name=newname
    if pivot is None:
        pts=[ob.matrix_world @ mathutils.Vector(c) for c in ob.bound_box]
        pivot=tuple(sum(p[i] for p in pts)/8 for i in range(3))
    bpy.context.scene.cursor.location=pivot
    bpy.ops.object.origin_set(type='ORIGIN_CURSOR')
    return ob
for k,(names,piv) in groups.items(): join(names,k,piv)
rest=[o.name for o in D.objects if o.type=='MESH' and o.name not in groups]
join(rest,"frame",(0,-0.19,-0.54))

# 6. For riders further away (pacers, ghosts): lighter still.
FAR=float(sys.argv[sys.argv.index("--")+3]) if len(sys.argv)>sys.argv.index("--")+3 else 1.0
if FAR<1.0:
    for name in ("frame","rear_wheel","front_wheel","crankset"):
        ob=D.objects[name]; m=ob.modifiers.new("Far","DECIMATE"); m.ratio=FAR
        bpy.ops.object.select_all(action='DESELECT'); ob.select_set(True); bpy.context.view_layer.objects.active=ob
        bpy.ops.object.convert(target='MESH')
# 7. Fit to VeloForge: the bottom bracket at (-0.02, 0.28), the saddle's top
# where the rider sits (SEAT_HEIGHT above it), x forward, y up, z right.
saddle=D.objects["frame"]  # saddle measured from its original part, kept below
BB=mathutils.Vector((0.0,-0.19,-0.54))
SADDLE_TOP_Z=0.985   # Cube.004: centre 0.93, half height 0.055
k=K
def vf(p):  # Blender world point -> VeloForge bike frame
    return mathutils.Vector(((p.y-BB.y)*k-0.02, (p.z-BB.z)*k+0.28, p.x*k))
def vfd(n):
    return mathutils.Vector((n.y, n.z, n.x))
import struct, math
pieces=[]
info={}
for name in ("frame","rear_wheel","front_wheel","crankset","pedal_right","pedal_left"):
    ob=D.objects[name]
    mw=ob.matrix_world; rot=mw.to_3x3().normalized()
    pivot=vf(mw.translation)
    me=ob.data; me.calc_loop_triangles()
    try: normals=[n.vector.copy() for n in me.corner_normals]
    except AttributeError:
        me.calc_normals_split(); normals=[l.normal.copy() for l in me.loops]
    looks=[]
    for mat in me.materials:
        if mat and mat.use_nodes:
            b=next((n for n in mat.node_tree.nodes if n.type=='BSDF_PRINCIPLED'),None)
            if b:
                c=b.inputs['Base Color'].default_value; r=b.inputs['Roughness'].default_value
                looks.append(((c[0],c[1],c[2]), 3.0 if r<0.4 else 0.0)); continue
        looks.append(((0.05,0.05,0.05),0.0))
    if not looks: looks=[((0.05,0.05,0.05),0.0)]
    # Wheels: sized so the tyre touches the ground (radius = hub height).
    wscale=1.0
    if name.endswith("wheel"):
        rmax=max(((mw@v.co)-mw.translation).length for v in me.vertices)*k
        wscale=pivot.y/rmax
        info[name]=(round(rmax,3),round(wscale,3))
    verts=[]
    for t in me.loop_triangles:
        col,mat=looks[min(t.material_index,len(looks)-1)]
        # The frame's paint is the rider's to choose: written as -1, for
        # VeloForge to fill with their colour.
        if all(abs(a-b)<1e-3 for a,b in zip(col,PAINT)):
            col=(-1.0,-1.0,-1.0)
        for li in t.loops:
            v=me.vertices[me.loops[li].vertex_index].co
            p=vf(mw@v)-pivot
            p*=wscale
            n=vfd((rot@normals[li]).normalized())
            verts.extend([p.x,p.y,p.z,n.x,n.y,n.z,col[0],col[1],col[2],mat])
    pieces.append((name,pivot,verts))
# 8. What the rider needs from this bike: crank length and its angle as
# modelled, and where the right hand holds the hood.
pr=[p for p in pieces if p[0]=="pedal_right"][0][1]
crank=math.hypot(pr.x+0.02, pr.y-0.28)
th0=math.atan2(pr.x+0.02, pr.y-0.28)
hood=D.objects.get("Cube.011")
# Cube.011 was joined into the frame: use its measured box (right hood).
hood_c=mathutils.Vector((0.375,1.04,0.62)); hood_top=0.62+0.11
grip=vf(mathutils.Vector((hood_c.x,hood_c.y,hood_top)))
out=sys.argv[sys.argv.index("--")+1]
with open(out+("/bike.vfb" if FAR>=1.0 else "/bike-far.vfb"),"wb") as f:
    f.write(b"VFBK"); f.write(struct.pack("<I",1))
    f.write(struct.pack("<5f",crank,th0,grip.x,grip.y,grip.z))
    f.write(struct.pack("<I",len(pieces)))
    for name,pivot,verts in pieces:
        nb=name.encode(); f.write(struct.pack("<B",len(nb))); f.write(nb)
        f.write(struct.pack("<3f",pivot.x,pivot.y,pivot.z)); f.write(struct.pack("<I",len(verts)//10))
        f.write(struct.pack("<%df"%len(verts),*verts))
print("FIT k=%.4f crank=%.4f th0=%.3f grip=(%.3f,%.3f,%.3f)"%(k,crank,th0,grip.x,grip.y,grip.z))
print("WHEELS", info)
print("PIECES", [(n, tuple(round(c,3) for c in p), len(v)//30) for n,p,v in pieces], "tris", sum(len(v)//30 for _,_,v in pieces))
# Preview of the lighter bike, side on.
scene=bpy.context.scene
for eng in ('BLENDER_EEVEE','BLENDER_EEVEE_NEXT','CYCLES'):
    try: scene.render.engine=eng; break
    except Exception: pass
scene.render.resolution_x=1400; scene.render.resolution_y=820
world=D.worlds[0] if D.worlds else D.worlds.new("w"); scene.world=world; world.use_nodes=True
bg=world.node_tree.nodes.get('Background')
if bg: bg.inputs[0].default_value=(0.55,0.6,0.66,1)
sun=D.objects.new("Sun", D.lights.new("Sun",'SUN')); scene.collection.objects.link(sun)
sun.rotation_euler=(math.radians(50),math.radians(10),math.radians(140)); sun.data.energy=3.5
cam=D.objects.new("Cam", D.cameras.new("Cam")); scene.collection.objects.link(cam); scene.camera=cam
cam.location=(7.0,0.0,0.05); cam.rotation_euler=(mathutils.Vector((0,0,0.05))-cam.location).to_track_quat('-Z','Y').to_euler()
cam.data.type='ORTHO'; cam.data.ortho_scale=3.1
scene.render.filepath=out+("/side-light.png" if FAR>=1.0 else "/side-far.png"); bpy.ops.render.render(write_still=True)
