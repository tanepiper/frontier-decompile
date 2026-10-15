---
name: frontier-verify
description: Verify Frontier reimplementation changes — cargo tests (with game-data skip behaviour), the WASM build, and side-by-side captures against the original running in DOSBox. Use after any port or format change and before committing.
---

# Verify a change

Prefer evidence you can look at: a test result, or a PNG you opened with the Read tool. Never
report "works" from a build alone.

```bash
cargo test --release                       # check output: game-data tests print "skipping" when absent
cargo build --release -p frontier-app --target wasm32-unknown-unknown
```

## Reference captures from the original

Run `tools/dosbox/run.sh` in the background, find the DOSBox window id, and capture it:

```bash
swift - <<'S'
import CoreGraphics
for w in CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as! [[String: Any]]
  where (w["kCGWindowOwnerName"] as? String ?? "").contains("dosbox") && (w["kCGWindowName"] as? String ?? "") != "" {
  print(w["kCGWindowNumber"]!) }
S
screencapture -x -o -l <id> game-data/derived/captures/<name>.png
```
DOSBox's own screenshot key (Ctrl+F5) saves the raw 320×200 frame, which is better for pixel
comparison than a window capture with the CRT shader. Captures stay in `game-data/derived/`.

Report exactly what ran: test names with pass/skip counts, capture paths you opened, and every
difference you saw.
