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
python3 - "$here/template.html" "$wasm" "$out" "$build" <<'PY'
import base64, sys
tpl, wasm, out, build = sys.argv[1:]
page = open(tpl).read()
page = page.replace("__WASM__", base64.b64encode(open(wasm, "rb").read()).decode())
page = page.replace("__BUILD__", build)
open(out, "w").write(page)
PY
echo "wrote $out"
