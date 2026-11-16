# Model lab

A web page for looking at VeloForge's rider, bike and scenery models before
a change goes into the app: turn the view, set the pose (pedalling, standing,
tucking, leaning into a corner, gradient, effort) and switch between the
trees, bushes, rocks and houses.

It runs the app's own model code: `src/gfx/math.rs`, `mesh.rs` and `body.rs`
are compiled as they are to WebAssembly, so the page shows exactly the
shapes and poses the app draws (with simpler lighting).

The **Ride** tab rides 1.6 km of road with grass and trees, using the app's
own grass and tree vertex shaders (taken from `src/gfx/shaders.rs` when the
page is built) and moving the terrain window on as the app does, so how the
meadow and trees are placed, thinned and move in the wind as you ride is
what the app does, bugs included. It can switch to the shaders of the
released app (`main`) to compare a fix with the bug, cap the frame rate (to
see a slow graphics card's view), and jump to just before the land moves on.

For automated checks the page has `window.lab`: set `lab.manual = true`,
then `lab.step(dt, still)` renders one frame (`still` holds the wind);
`lab.holdView`, `lab.pickShaders("base" | "now")` and `lab.ride()` let a
script hold the picture still and compare the two shader versions.

```bash
tools/model-lab/build.sh            # writes tools/model-lab/model-lab.html
```

Open the page in a browser. It needs Rust with the `wasm32-unknown-unknown`
target (the script adds it) and loads three.js from a CDN.
