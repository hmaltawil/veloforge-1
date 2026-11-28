#!/usr/bin/env bash
# Build the model lab page: the app's model code compiled to WebAssembly,
# inlined into template.html. Writes model-lab.html (or the path given).
#   tools/model-lab/build.sh [OUT.html]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/model-lab.html}"
cd "$here"
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
now_cfg=""
if [ -f "$here/../../src/gfx/buildings.rs" ]; then now_cfg="--cfg lab_buildings"; fi
if grep -q "pub fn oak" "$here/../../src/gfx/mesh.rs"; then now_cfg="$now_cfg --cfg lab_oak"; fi
if [ -f "$here/../../src/gfx/fields.rs" ]; then now_cfg="$now_cfg --cfg lab_fields"; fi
if [ -f "$here/../../src/gfx/mountains.rs" ]; then now_cfg="$now_cfg --cfg lab_mountains"; fi
if [ -f "$here/../../src/gfx/japanese.rs" ]; then now_cfg="$now_cfg --cfg lab_japanese"; fi
if grep -q "volcano" "$here/../../src/gfx/mountains.rs" 2>/dev/null; then now_cfg="$now_cfg --cfg lab_volcano"; fi
if [ -f "$here/../../src/gfx/aero_bike.rs" ]; then now_cfg="$now_cfg --cfg lab_aero"; fi
RUSTFLAGS="$now_cfg" cargo build --release --target wasm32-unknown-unknown --quiet
wasm="$here/target/wasm32-unknown-unknown/release/model_lab.wasm"
# The released models too (main's model code with this page's exports), so
# the page can switch between them and this build's.
base_dir="$(mktemp -d)"
mkdir -p "$base_dir/src/gfx" "$base_dir/tools/model-lab"
for f in math mesh body impostor fields; do
    git -C "$here" show "main:src/gfx/$f.rs" >"$base_dir/src/gfx/$f.rs" 2>/dev/null \
        || cp "$here/../../src/gfx/$f.rs" "$base_dir/src/gfx/$f.rs"
done
# Farm buildings and mountains, in releases that have them.
for f in buildings mountains; do
    git -C "$here" show "main:src/gfx/$f.rs" >"$base_dir/src/gfx/$f.rs" 2>/dev/null \
        || rm -f "$base_dir/src/gfx/$f.rs"
done
cp -r "$here/Cargo.toml" "$here/Cargo.lock" "$here/src" "$base_dir/tools/model-lab/"
cp "$here/../../rust-toolchain.toml" "$base_dir/"   # the same Rust as this build
# Releases before 1.6.8 had middle-distance trees: draw them with those.
base_cfg=""
if grep -q "pub fn broadleaf_mid" "$base_dir/src/gfx/mesh.rs"; then base_cfg="--cfg lab_mid"; fi
if [ -f "$base_dir/src/gfx/buildings.rs" ]; then base_cfg="$base_cfg --cfg lab_buildings"; fi
if grep -q "pub fn oak" "$base_dir/src/gfx/mesh.rs"; then base_cfg="$base_cfg --cfg lab_oak"; fi
if git -C "$here" cat-file -e main:src/gfx/fields.rs 2>/dev/null; then base_cfg="$base_cfg --cfg lab_fields"; fi
if [ -f "$base_dir/src/gfx/mountains.rs" ]; then base_cfg="$base_cfg --cfg lab_mountains"; fi
if grep -q "volcano" "$base_dir/src/gfx/mountains.rs" 2>/dev/null; then base_cfg="$base_cfg --cfg lab_volcano"; fi
(cd "$base_dir/tools/model-lab" && CARGO_TARGET_DIR="$here/target-base" RUSTFLAGS="$base_cfg" \
    cargo build --release --target wasm32-unknown-unknown --quiet)
wasm_base="$here/target-base/wasm32-unknown-unknown/release/model_lab.wasm"
rm -rf "${base_dir:?}"
# The Mountains tab's scenes: the app's own land around a few routes, baked
# by the app's `lab_scenes` test (see src/gfx/mod.rs).
scenes_dir="$(mktemp -d)"
if grep -q "fn lab_scenes" "$here/../../src/gfx/mod.rs"; then
    (cd "$here/../.." && VELOFORGE_LAB_OUT="$scenes_dir" cargo test --release --quiet lab_scenes -- --ignored >/dev/null)
fi
build="$(git -C "$here" rev-parse --abbrev-ref HEAD) @ $(git -C "$here" rev-parse --short HEAD)"
if ! git -C "$here/../.." diff --quiet -- src/gfx; then build="$build + changes"; fi
# The released shaders too (main), so the lab can show a bug next to its fix.
base_rs="$(mktemp)"; trap 'rm -f "$base_rs"' EXIT
git -C "$here" show main:src/gfx/shaders.rs >"$base_rs" 2>/dev/null || cp "$here/../../src/gfx/shaders.rs" "$base_rs"
base_ver="$(git -C "$here" show main:Cargo.toml 2>/dev/null | sed -n 's/^version = "\(.*\)"/\1/p' | head -1)"
python3 - "$here/template.html" "$wasm" "$out" "$build" "$base_rs" "${base_ver:-main}" "$wasm_base" "$scenes_dir" "${LAB_SHOTS:-}" <<'PY'
import base64, glob, os, sys
tpl, wasm, out, build, base_rs, base_ver, wasm_base, scenes_dir, shots = sys.argv[1:]
page = open(tpl).read()
page = page.replace("__WASM_BASE__", base64.b64encode(open(wasm_base, "rb").read()).decode())
page = page.replace("__WASM__", base64.b64encode(open(wasm, "rb").read()).decode())
page = page.replace("__BUILD__", build)
# The app's own wind, so the lab's trees and grass move exactly as in the app.
src = open(os.path.join(os.path.dirname(tpl), "../../src/gfx/shaders.rs")).read()
def const(name, src=src):
    if "pub const %s: &str" % name not in src:
        return None
    i = src.index("pub const %s: &str = r#\"" % name)
    i = src.index("\n", i) + 1
    return src[i:src.index('"#;', i)]
noise = const("NOISE")
helpers = noise[noise.index("float hash12"):noise.index("float fbm(")]
page = page.replace("/*__WIND_GLSL__*/", (helpers + const("WIND")).replace("`", "'").replace("${", "$ {"))
# The app's own shaders for the ride: grass and trees run exactly as there.
import json
names = ["NOISE", "WIND", "GRASS_VS", "OBJECT_VS", "FOLIAGE", "GROUND_MAP",
         "LOD_FS", "IMPOSTOR_VS", "IMPOSTOR_SURFACE", "LIGHT", "SCREEN_VS", "SKY_FS",
         "TERRAIN_VS", "TERRAIN_FS", "MOUNTAIN_RIBS", "MOUNTAIN_VS", "MOUNTAIN_FS"]
glsl = {k: const(k) for k in names}
base = open(base_rs).read()
glsl["BASE"] = {k: const(k, base) for k in names}
glsl["BASE_VERSION"] = base_ver
page = page.replace("__APP_GLSL_JSON__", json.dumps(glsl))
# The Mountains tab's scenes, and screenshots of the app at each if given
# (LAB_SHOTS=<dir> holding <n>-before.jpg and <n>-after.jpg, n from 0).
meta = os.path.join(scenes_dir, "scenes.json")
page = page.replace("__MOUNTAIN_SCENES_JSON__", open(meta).read() if os.path.exists(meta) else "null")
binp = os.path.join(scenes_dir, "scenes.bin")
page = page.replace("__MOUNTAIN_SCENES_BIN__", base64.b64encode(open(binp, "rb").read()).decode() if os.path.exists(binp) else "")
pics = {}
for f in sorted(glob.glob(os.path.join(shots, "*.jpg"))) if shots else []:
    pics[os.path.basename(f)[:-4]] = "data:image/jpeg;base64," + base64.b64encode(open(f, "rb").read()).decode()
page = page.replace("__MOUNTAIN_SHOTS__", json.dumps(pics))
open(out, "w").write(page)
PY
rm -rf "${scenes_dir:?}"
echo "wrote $out"
