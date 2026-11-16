#!/usr/bin/env bash
# Build the model lab page: the app's model code compiled to WebAssembly,
# inlined into template.html. Writes model-lab.html (or the path given).
#   tools/model-lab/build.sh [OUT.html]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/model-lab.html}"
cd "$here"
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
cargo build --release --target wasm32-unknown-unknown --quiet
wasm="$here/target/wasm32-unknown-unknown/release/model_lab.wasm"
build="$(git -C "$here" rev-parse --abbrev-ref HEAD) @ $(git -C "$here" rev-parse --short HEAD)"
if ! git -C "$here/../.." diff --quiet -- src/gfx; then build="$build + changes"; fi
# The released shaders too (main), so the lab can show a bug next to its fix.
base_rs="$(mktemp)"; trap 'rm -f "$base_rs"' EXIT
git -C "$here" show main:src/gfx/shaders.rs >"$base_rs" 2>/dev/null || cp "$here/../../src/gfx/shaders.rs" "$base_rs"
base_ver="$(git -C "$here" show main:Cargo.toml 2>/dev/null | sed -n 's/^version = "\(.*\)"/\1/p' | head -1)"
python3 - "$here/template.html" "$wasm" "$out" "$build" "$base_rs" "${base_ver:-main}" <<'PY'
import base64, os, sys
tpl, wasm, out, build, base_rs, base_ver = sys.argv[1:]
page = open(tpl).read()
page = page.replace("__WASM__", base64.b64encode(open(wasm, "rb").read()).decode())
page = page.replace("__BUILD__", build)
# The app's own wind, so the lab's trees and grass move exactly as in the app.
src = open(os.path.join(os.path.dirname(tpl), "../../src/gfx/shaders.rs")).read()
def const(name, src=src):
    i = src.index("pub const %s: &str = r#\"" % name)
    i = src.index("\n", i) + 1
    return src[i:src.index('"#;', i)]
noise = const("NOISE")
helpers = noise[noise.index("float hash12"):noise.index("float fbm(")]
page = page.replace("/*__WIND_GLSL__*/", (helpers + const("WIND")).replace("`", "'").replace("${", "$ {"))
# The app's own shaders for the ride: grass and trees run exactly as there.
import json
glsl = {k: const(k) for k in ["NOISE", "WIND", "GRASS_VS", "OBJECT_VS"]}
base = open(base_rs).read()
glsl["BASE"] = {k: const(k, base) for k in ["NOISE", "WIND", "GRASS_VS", "OBJECT_VS"]}
glsl["BASE_VERSION"] = base_ver
page = page.replace("__APP_GLSL_JSON__", json.dumps(glsl))
open(out, "w").write(page)
PY
echo "wrote $out"
