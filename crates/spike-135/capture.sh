#!/usr/bin/env bash
# usage: capture.sh MODE OUT_DIR   (MODE is anchor|pixels)
# Headless: captures the same view before and after ~300 remeasuring frames.
# WORK holds a one-note vault/. Run from the repository root after building
# jade-app --features debug-socket and jade-debug-cli in release.
: "${WORK:?set WORK to the working directory}"
mode=$1 out=$2
J="$PWD/target/release/jade-debug --socket /tmp/jade-spike135.sock"
log=$out/$mode.log; rm -f "$log"
JADE_SPIKE_SCROLL=$mode JADE_SPIKE_LOG=$log $J launch "$WORK/vault" --headless --size 900x600 >/dev/null || exit 1
shot() { $J capture editor --rect 0,0,"$width",400 --out "$out/$mode-$1.png" >/dev/null; }
$J click editor >/dev/null
$J key m >/dev/null
sleep 3   # let the scrollbar fade
width=$($J targets | awk '$1=="editor" {print int($4) - 20}')
shot before
$J capture editor --out "$out/$mode-full-before.png" --scale 0.5 >/dev/null
$J key r >/dev/null
for _ in $(seq 100); do $J key x >/dev/null 2>&1; done
sleep 3
$J key x >/dev/null 2>&1
shot after
$J capture editor --out "$out/$mode-full-after.png" --scale 0.5 >/dev/null
$J quit >/dev/null
if cmp -s "$out/$mode-before.png" "$out/$mode-after.png"; then same=identical; else same=different; fi
first=$(grep -m1 'anchor=' "$log"); echo "$mode: captures $same, frames logged $(grep -c '^frame=' "$log")"
python3 - "$log" <<'PY'
import re, sys
rows = [dict(re.findall(r"(\w+)=(\S+)", l)) for l in open(sys.argv[1]) if l.startswith("frame=")]
tail = rows[-250:]
for key in ("above_px", "total_px", "tracked_y"):
    v = [float(r[key]) for r in tail]
    print(f"  {key}: {min(v):.1f} .. {max(v):.1f}")
print("  anchors:", len({r["anchor"] for r in tail}))
PY
