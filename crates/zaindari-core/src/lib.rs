//! zaindari-core — pure logic for the zaindari orchestrator.
//!
//! zaindari is one CLI over three LLM-trust engines:
//!   * **Gate** ([`adapters::gate`]) — pre-ship eval regression gate (`aatxe`).
//!   * **Guard** ([`adapters::guard`]) — runtime rule packs (`iratxo`).
//!   * **Watch** ([`adapters::watch`]) — post-ship drift detection (`cardinal-map`).
//!
//! Everything here is engine-agnostic once an adapter has mapped raw engine
//! output into the shared [`report::ZaindariReport`] model. The binary crate
//! (`zaindari-cli`) is a thin wrapper that parses args, calls [`run::run`],
//! renders via [`render`], and exits via [`policy::exit_code`].

pub mod adapters;
pub mod config;
pub mod engine;
pub mod policy;
pub mod render;
pub mod report;
pub mod run;

pub use config::{Config, ConfigError};
pub use policy::{exit_code, PolicyOptions};
pub use report::{
    Finding, Metric, MetricDirection, PillarResult, PillarStatus, Pillars, Severity, ToolVersions,
    ZaindariReport, REPORT_SCHEMA_VERSION,
};
pub use run::{run, RunContext, Selection};
