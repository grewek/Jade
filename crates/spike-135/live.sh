#!/usr/bin/env bash
# usage: live.sh LOG
# Live window (keep it visible): synthetic flings with and without
# remeasuring, then remeasuring at rest. WORK holds a one-note vault/.
: "${WORK:?set WORK to the working directory}"
log=$1; rm -f "$log"
J="$PWD/target/release/jade-debug --socket /tmp/jade-spike135.sock"
JADE_SPIKE_SCROLL=anchor JADE_SPIKE_LOG=$log $J launch "$WORK/vault" >/dev/null || exit 1
step() { $J "$@" >/dev/null || echo "FAILED: $*"; }
phase() { echo "## $1" >> "$log"; }
step click editor
step key m
flings() { for _ in 1 2 3; do step key f; sleep 4; step key b; sleep 4; done; }
sleep 1
phase fling; flings
step key r
phase fling+remeasure; flings
phase remeasure-at-rest; sleep 8
step key r
phase end
$J quit >/dev/null
