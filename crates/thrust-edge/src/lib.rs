//! `thrust-edge`: the Worker on the shared hostname (ADR-0111).
//!
//! A request without a valid pin goes to production through the service binding `THRUST`. A pinned request goes to the target
//! Flagship allows, a Worker Preview of `thrust` fetched over its workers.dev URL with an Access service token, or else to
//! production. The edge holds no application code and no data binding.
//!
//! The decision logic is in plain modules, built and tested natively: [`pin`] (the opt-in cookie), [`access`] (the Access token
//! on an opt-in path), [`route`] (what may go on, and where) and [`forward`] (the upstream request and the response). Only the
//! `runtime` module, compiled for wasm32 alone, touches the Workers runtime.

pub mod access;
pub mod forward;
pub mod pin;
pub mod route;

/// The fuzz harnesses (tests and fuzz builds only): `fuzz/` runs campaigns on them, `fuzz_replay_` tests replay the corpus.
#[cfg(any(test, fuzzing))]
pub mod fuzz_harness;

#[cfg(target_arch = "wasm32")]
mod runtime;
