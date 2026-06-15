//! Engine adapters. Each module maps one engine's real output into the
//! shared [`crate::report`] model. Parsing is pure (testable against
//! fixtures); the orchestrator in [`crate::run`] handles invocation.

pub mod gate;
pub mod guard;
pub mod watch;
