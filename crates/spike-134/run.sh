#!/usr/bin/env bash
# usage: run.sh MODE FIXTURE LOG [--headless]
# MODE is measured|fill. WORK holds fx/ (from make_fixtures.py) and a one-note
# vault/. APP overrides the binary, CAPTURE saves a PNG (headless only).
# Run from the repository root after building jade-app --features debug-socket
# and jade-debug-cli in release.
: "${WORK:?set WORK to the working directory}"
J=$PWD/target/release/jade-debug
mode=$1 fixture=$2 log=$3; shift 3
rm -f "$log"
export JADE_DEBUG_SOCKET=/tmp/jade-spike134.sock
JADE_SPIKE_LAYOUT="$WORK/fx/$fixture" JADE_SPIKE_MODE=$mode JADE_SPIKE_LOG="$log" \
  $J --socket $JADE_DEBUG_SOCKET launch $WORK/vault ${APP:+--bin $APP} "$@" || exit 1
step() { echo "## $*" >> "$log"; $J "$@" >/dev/null || echo "FAILED: $*"; sleep 0.4; }
step click editor
for i in 1 2 3 4 5 6; do step click toggle-sidebar; done
step click editor
for w in left left left right right right; do step key ctrl+alt+$w; done
step type "typing in the spike "
step key enter
step type "héllo wörld"
step key backspace
step key tab
step scroll editor -4000
for i in 1 2 3; do step click toggle-sidebar; done
$J targets | grep -E "editor|sidebar "
[ -n "$CAPTURE" ] && { step scroll editor 100000; $J capture editor --out "$CAPTURE"; }
$J quit
