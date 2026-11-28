# Model lab

A web page for looking at VeloForge's rider, bike and scenery models before
a change goes into the app: turn the view, set the pose (pedalling, standing,
tucking, leaning into a corner, gradient, effort) and switch between the
trees, bushes, rocks and houses. The Rider tab can show the classic bike
or the aero bike (`src/gfx/aero_bike.rs`, also as seen far off) in any
frame colour, and hide the rider to see the bike on its own.

It runs the app's own model code: `src/gfx/math.rs`, `mesh.rs` and `body.rs`
are compiled as they are to WebAssembly, so the page shows exactly the
shapes and poses the app draws (with simpler lighting).

The **Ride** tab rides 1.6 km of road with grass and trees, using the app's
own grass and tree vertex shaders (taken from `src/gfx/shaders.rs` when the
page is built) and moving the terrain window on as the app does, so how the
meadow and trees are placed, thinned and move in the wind as you ride is
what the app does, bugs included. It can cap the frame rate (to see a slow
graphics card's view) and jump to just before the land moves on.

Both the Ride and Scenery tabs can switch between this build and the
released app (`main`): `build.sh` also compiles `main`'s model code and
takes `main`'s shaders, so a change to the trees, grass or their shaders
can be compared with what it changes. Scenery is drawn with the app's own
tree shader and leaf lighting (without its shadows).

Trees are drawn as the app draws them at the distance they are seen from:
full models, middle ones, then cards whose pictures the page makes with the
app's own `impostor.rs`, each tree dissolving from one level to the next.
The Scenery tab's distance slider walks away from a tree (keeping it the
same size on screen if asked) to watch it go through every level.

The **Mountains** tab shows the land to the horizon at a few spots (three
on the imported Alpe d'Huez, and a route of each scenery): the app's own
near and distant land, baked by the app's `lab_scenes` test when the page is
built, with the mountains' mesh beyond built in the page by the app's
`mountains.rs`, all drawn with the app's own sky, land and mountain shaders
in the light of a clear day (without shadow maps, which do not reach that
far, or the finishing pass). Drag to look round. `LAB_SHOTS=<dir>` adds
screenshots of the app at each spot (`<n>-before.jpg`, `<n>-after.jpg`).

For automated checks the page has `window.lab`: set `lab.manual = true`,
then `lab.step(dt, still)` renders one frame (`still` holds the wind);
`lab.holdView`, `lab.pickShaders("base" | "now")` and `lab.ride()` let a
script hold the picture still and compare the two shader versions;
`window.lab_mountains` (`scene`, `look`, `set`, `sun`, `show`, `render`)
sets up the Mountains view.

```bash
tools/model-lab/build.sh            # writes tools/model-lab/model-lab.html
```

Open the page in a browser. It needs Rust with the `wasm32-unknown-unknown`
target (the script adds it) and loads three.js from a CDN.
