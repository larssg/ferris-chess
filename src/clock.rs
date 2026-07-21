//! Monotonic clock that works on native and wasm32 targets.
//!
//! `std::time::Instant::now()` panics on `wasm32-unknown-unknown` because the
//! target has no OS clock, so wasm builds substitute `web_time::Instant`, a
//! drop-in replacement backed by the browser's `performance.now()`. On native
//! targets this module is a pure re-export of the std type — zero overhead and
//! zero behavior change. Front-ends should use this alias too so their timing
//! stays portable alongside the engine's.

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;

#[cfg(target_arch = "wasm32")]
pub use web_time::Instant;
