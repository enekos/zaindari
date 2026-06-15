//! The shared report model — the one shape every engine adapter maps into,
//! and the only thing the HTML renderer and exit-code policy understand.
//!
//! Each engine (aatxe / iratxo / cardinal-map) speaks its own JSON or text.
//! Adapters translate; everything downstream is engine-agnostic.

use serde::{Deserialize, Serialize};

/// Schema version of [`ZaindariReport`]. Bumped when the shape changes so a
/// stored run JSON can be re-rendered by a known-compatible renderer.
pub const REPORT_SCHEMA_VERSION: u32 = 1;

/// Top-level run report: one optional [`PillarResult`] per pillar plus the
/// engine version strings observed at run time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ZaindariReport {
    pub schema_version: u32,
    /// Engine binary -> version string (best-effort; empty when an engine was
    /// not invoked or did not report a version).
    pub tool_versions: ToolVersions,
    pub pillars: Pillars,
}

impl ZaindariReport {
    /// A report with every pillar absent. Adapters fill the pillars they run.
    pub fn empty() -> Self {
        Self {
            schema_version: REPORT_SCHEMA_VERSION,
            tool_versions: ToolVersions::default(),
            pillars: Pillars::default(),
        }
    }
}

/// Version strings keyed by pillar. `None` = that engine reported no version
/// (or was never run / was missing).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolVersions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<String>,
}

/// The three pillars. A `None` pillar was not configured; a `Some` with
/// [`PillarStatus::Skipped`] was configured but deliberately not run.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Pillars {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<PillarResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<PillarResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<PillarResult>,
}

/// Outcome of one pillar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PillarStatus {
    /// Engine ran and met its bar.
    Pass,
    /// Engine ran and failed its bar (regression / rule failure). CI-gating.
    Fail,
    /// Engine ran; result is informational and below a hard-fail line
    /// (e.g. watch anomalies under default policy).
    Warn,
    /// Pillar not configured, or configured-but-not-selected this run.
    Skipped,
    /// Engine binary could not be found / executed.
    EngineMissing,
}

impl PillarStatus {
    /// Human "traffic light" label used in text and HTML output.
    pub fn label(self) -> &'static str {
        match self {
            PillarStatus::Pass => "PASS",
            PillarStatus::Fail => "FAIL",
            PillarStatus::Warn => "WARN",
            PillarStatus::Skipped => "SKIPPED",
            PillarStatus::EngineMissing => "ENGINE MISSING",
        }
    }
}

/// Result for a single pillar, in the shared shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct PillarResult {
    pub status: PillarStatus,
    /// One plain-English sentence a non-engineer can read.
    pub headline: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<Metric>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
    /// Path to the raw engine output captured for this pillar, if written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_ref: Option<String>,
}

impl PillarResult {
    /// Construct a minimal result with just status + headline.
    pub fn new(status: PillarStatus, headline: impl Into<String>) -> Self {
        Self {
            status,
            headline: headline.into(),
            metrics: Vec::new(),
            findings: Vec::new(),
            raw_ref: None,
        }
    }

    /// Standard `engine_missing` result carrying an install hint as the
    /// headline. Adapters return this instead of panicking when the binary
    /// is absent.
    pub fn engine_missing(binary: &str, hint: &str) -> Self {
        Self::new(
            PillarStatus::EngineMissing,
            format!("engine `{binary}` not found on PATH — {hint}"),
        )
    }
}

/// Direction a metric should move for "better". Lets the renderer colour a
/// delta as good/bad without hard-coding per-metric knowledge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricDirection {
    /// Higher is better (recall, F1, AUC).
    HigherBetter,
    /// Lower is better (false positives, Brier, MAE).
    LowerBetter,
    /// No inherent good direction (counts, sizes).
    Neutral,
}

/// One named measurement, optionally compared to a baseline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Metric {
    pub name: String,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<f64>,
    /// `value - baseline`, present only when `baseline` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<f64>,
    pub direction: MetricDirection,
}

impl Metric {
    /// A metric with no baseline.
    pub fn plain(name: impl Into<String>, value: f64, direction: MetricDirection) -> Self {
        Self {
            name: name.into(),
            value,
            baseline: None,
            delta: None,
            direction,
        }
    }

    /// A metric with a baseline; `delta` is computed as `value - baseline`.
    pub fn with_baseline(
        name: impl Into<String>,
        value: f64,
        baseline: f64,
        direction: MetricDirection,
    ) -> Self {
        Self {
            name: name.into(),
            value,
            baseline: Some(baseline),
            delta: Some(value - baseline),
            direction,
        }
    }

    /// True when a baselined metric moved in the wrong direction.
    pub fn regressed(&self) -> bool {
        match (self.delta, self.direction) {
            (Some(d), MetricDirection::HigherBetter) => d < 0.0,
            (Some(d), MetricDirection::LowerBetter) => d > 0.0,
            _ => false,
        }
    }
}

/// Severity of a finding. Ordered worst-first for grouping in output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    Major,
    Minor,
    Nit,
    Info,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::Major => "major",
            Severity::Minor => "minor",
            Severity::Nit => "nit",
            Severity::Info => "info",
        }
    }

    /// Worst-to-best order for grouped rendering.
    pub fn all_worst_first() -> [Severity; 5] {
        [
            Severity::Critical,
            Severity::Major,
            Severity::Minor,
            Severity::Nit,
            Severity::Info,
        ]
    }
}

/// One actionable item surfaced by an engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Finding {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    /// Free-form locator (file:line, entity name, rule id) when the engine
    /// gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

impl Finding {
    pub fn new(severity: Severity, title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            severity,
            title: title.into(),
            detail: detail.into(),
            location: None,
        }
    }

    pub fn with_location(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_delta_is_value_minus_baseline() {
        let m = Metric::with_baseline("recall", 0.80, 0.90, MetricDirection::HigherBetter);
        assert!((m.delta.unwrap() - (-0.10_f64)).abs() < 1e-9);
        assert!(m.regressed());
    }

    #[test]
    fn lower_better_metric_regresses_when_it_rises() {
        let m = Metric::with_baseline("fp_per_case", 3.0, 2.0, MetricDirection::LowerBetter);
        assert!(m.regressed());
        let improved = Metric::with_baseline("fp_per_case", 1.0, 2.0, MetricDirection::LowerBetter);
        assert!(!improved.regressed());
    }

    #[test]
    fn plain_metric_never_regresses() {
        let m = Metric::plain("count", 5.0, MetricDirection::Neutral);
        assert!(!m.regressed());
    }

    #[test]
    fn report_roundtrips_through_json() {
        let mut r = ZaindariReport::empty();
        r.pillars.gate = Some(PillarResult::new(PillarStatus::Pass, "all good"));
        let json = serde_json::to_string(&r).unwrap();
        let back: ZaindariReport = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
