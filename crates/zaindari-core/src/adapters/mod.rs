//! Engine adapters. Each module maps one engine's real output into the
//! shared [`crate::report`] model. Parsing is pure (testable against
//! fixtures); the orchestrator in [`crate::run`] handles invocation.
//!
//! [`gate`]/[`guard`]/[`watch`] each speak one specific engine's JSON;
//! [`native`] speaks zaindari's own contract, so any command that emits the
//! envelope can drive a pillar regardless of language.

pub mod gate;
pub mod guard;
pub mod native;
pub mod watch;
