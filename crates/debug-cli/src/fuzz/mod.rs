//! Random but repeatable input for a headless app (#140). So far only the
//! choice of input; nothing runs it yet.

// The tests use everything; the runner that does comes next.
#![cfg_attr(not(test), expect(dead_code, reason = "no runner yet (#140)"))]

mod actions;
mod rng;
