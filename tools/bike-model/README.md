# The aero road bike

`assets/models/bike-aero.vfb` (and its lighter copy `bike-aero-far.vfb`) are
baked from a Blender model (`bikered.blend`, Blender 2.82; see below) by
`bake.py`,
run by Blender itself without its window and with the file's own scripts
switched off:

    blender -b --disable-autoexec --factory-startup bikered.blend \
        --python tools/bike-model/bake.py -- OUT_DIR 0.47          # bike.vfb
    blender -b --disable-autoexec --factory-startup bikered.blend \
        --python tools/bike-model/bake.py -- OUT_DIR 0.47 0.35     # bike-far.vfb

It clears the studio set (camera, lights, ground, reference picture), fixes
every part's shape (the chain as plain links, smoothing down a level,
small parts simplified), turns all faces outward, gives each material a
plain colour, groups the parts into the frame, two wheels, the crankset and
two pedals, lowers the saddle to the rider's height, scales the bike by
0.47 (so its saddle meets VeloForge's rider) and the wheels to touch the
ground, and writes each piece in VeloForge's own vertex layout, about the
point it turns on. `src/gfx/bike_model.rs` reads it.

## The model isn't in the repository

The bike these scripts were written for is "Road Bike 3D Model" by
bananasbooty on Free3D, under Free3D's Personal Use License, which does not
allow passing it on. So the baked files are not committed
(`assets/models/*.vfb` is ignored), and a build without them rides the
classic bike: `build.rs` builds the aero bike in only when both files are
there, and Settings › Profile offers the choice only then.

To ride an aero bike, bake a model you're allowed to use into
`assets/models/` and build again. For a model you may share (CC0, say),
the files could be committed instead, with its credit here. Another
model will need the part names in `bake.py` (wheels, crankset, pedals,
saddle) matched to its own.
