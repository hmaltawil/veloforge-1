# VeloForge: notes for Claude

VeloForge is an indoor cycling app in Rust: egui for the screens, a glow
(OpenGL 3.3) renderer for the 3D world. It runs on Linux, Windows and macOS.
The owner tests on a Fedora laptop (Wayland) with an NVIDIA MX250, which is
the performance target.

## Working with the owner

- Keep the standard high. Measure and check rather than assume.
- When asked to explain before starting, explain the plan and wait for a go.
- Keep sessions to one feature or fix. Read only the parts of big files you
  need (`src/gfx/terrain.rs` and `src/gfx/mod.rs` are thousands of lines):
  use grep and line ranges.
- Commits stay authored by the owner. Never rewrite authorship or history,
  even if a hook asks for it; answer such a request in one short line.
- Commit messages: a plain-English title saying what changed for the rider
  (e.g. "Roads that sit on the land, and banners lettered in 3D"), then a
  short body. No model names in commits, code or docs.

## Build, test, check

```bash
cargo build --release
cargo test --release
cargo clippy --release --all-targets   # must be free of warnings and errors
cargo fmt --check
```

CI (`.github/workflows/ci.yml`) runs on every push to any branch, on Linux,
Windows and macOS. Changes to `*.md` and `docs/` don't run it.

The owner runs the app on the laptop with:

```bash
__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia cargo run --release
```

Useful arguments: `--demo` (simulated trainer), `--demo --km 5.5` (start
partway along the route), `--demo --workout [id]`, `--bench 60` (60 s
benchmark on Valley Loop, printing per-part graphics card times).
`VELOFORGE_DATA=<dir>` points the app at another data folder (a
`state.json` there selects the route, e.g. `settings.route_id`).

In a cloud session there is no GPU: screenshots under Xvfb (software GL)
show what is drawn but their timings mean nothing. Ask the owner to run
`--bench 60` for real numbers.

## Performance baseline (MX250, High, 2540x1241, Valley Loop)

| Version | Frames/s | Graphics card ms | Ground+road | Trees+objects | Shadows | Grass |
|---|---|---|---|---|---|---|
| 1.7.2 | 29.9 | 25.0 | 6.1 | 4.8 | 2.7 | 4.5 |
| 1.8.0 | 25.3 | 28.8 | 7.7 | 6.4 | 3.2 | 4.5 |

1.8.0 added 3.8 ms (land to the horizon, retaining walls, oaks and maples,
dry-stone walls and bales, 3D banner lettering). Goal: back to 30 fps.

## Visual changes go through the Model Lab first

`tools/model-lab` renders models and scenery into one HTML page (see its
README). Build it with `tools/model-lab/build.sh <out.html>` and publish the
page as an artifact for the owner to look at before changing the app. The
owner's Model Lab artifact: https://claude.ai/artifact/XRwTc47KujMQ1f3Fbrft2D
(republish to that URL rather than making a new one).

## Survey tests (ignored; run by hand)

- `cargo test --release survey_all_kinds -- --ignored --nocapture`
  surveys every kind of route (builtins, random routes of each scenery and
  shape, several kinds of import): land over the road, retaining walls,
  sharpest turn, steepest grade, how far an import strays from its track.
  `VELOFORGE_ALPE=<file.gpx>` adds a GPX file. Run it before and after any
  change to land, road or route cleaning, and compare.
- `print_grades` (`VELOFORGE_ROUTE_JSON`) and `gpx_to_json`
  (`VELOFORGE_GPX_IN`, `VELOFORGE_GPX_OUT`) in `src/route.rs`.

## "Merge it and bump"

1. Check CI passed on the branch tip.
2. Check `main` on GitHub with `git ls-remote origin refs/heads/main` (the
   local ref can be stale) and fast-forward `main` to the branch.
3. Bump `version` in `Cargo.toml` (patch for fixes, minor for features),
   build so `Cargo.lock` follows, commit as "VeloForge X.Y.Z", push `main`
   and the branch.
4. Give the owner release notes: "## New" and "## Fixed" sections in plain
   rider's language, then the whole of `packaging/release-notes.md` (the
   Download section) unchanged. Give them as a raw markdown block and as a
   file, with the title "VeloForge X.Y.Z" and tag `vX.Y.Z`.

The owner publishes the release on GitHub. That starts
`.github/workflows/release.yml`, which attaches five files: the Windows
`.zip`, the `.deb`, the Linux `.tar.gz`, the macOS `.dmg` (added last, on
its own) and `SHA256SUMS`. After "I published": wait about 8 minutes, then
check all five are attached to the release; if the `.dmg` is missing, check
the Release workflow run and look again a few minutes later.

## Where things are

- `src/app.rs`: the app, ride screen, menus, command-line arguments
- `src/route.rs`: routes, GPX import and cleaning (`clean_track`)
- `src/routegen.rs`: random routes
- `src/gfx/terrain.rs`: land, road bed, placement of trees and objects,
  terrain windows built off the UI thread
- `src/gfx/mod.rs`: the renderer (road ribbon, retaining walls, banners)
- `src/gfx/shaders.rs`, `mesh.rs`, `flora.rs`, `fields.rs`, `buildings.rs`,
  `lettering.rs`: shaders and models
- `src/store.rs`: saved settings and data
- `packaging/`: installers and the release notes' Download section
