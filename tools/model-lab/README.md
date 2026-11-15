# Model lab

A web page for looking at VeloForge's rider, bike and scenery models before
a change goes into the app: turn the view, set the pose (pedalling, standing,
tucking, leaning into a corner, gradient, effort) and switch between the
trees, bushes, rocks and houses.

It runs the app's own model code: `src/gfx/math.rs`, `mesh.rs` and `body.rs`
are compiled as they are to WebAssembly, so the page shows exactly the
shapes and poses the app draws (with simpler lighting, and without the
shaders' wind and colour effects).

```bash
tools/model-lab/build.sh            # writes tools/model-lab/model-lab.html
```

Open the page in a browser. It needs Rust with the `wasm32-unknown-unknown`
target (the script adds it) and loads three.js from a CDN.
