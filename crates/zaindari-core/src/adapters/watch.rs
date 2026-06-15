//! Watch adapter — wraps `cardinal-map check`.
//!
//! Engine contract (observed in cardinal-map @ cmd/check.go + internal/score):
//!   * `cardinal-map check --json --schema <s> --profiles <dir> --input <names.json> --threshold <t>`
//!   * `--json` emits a JSON array of `score.Result`:
//!     `{ name, entityType, cardinality, zscore, mode, threshold, flagged,
//!     dimensions: [{ name, kind, score, reason, value }] }`
//!   * a name missing from the trained profiles scores cardinality 1.0,
//!     flagged=true, with a `_resolution` meta dimension.
//!   * exit code: 0 on success; non-zero only on operational error (bad schema,
//!     unreadable input) — anomalies do NOT make it exit non-zero.
//!
//! Policy: anomalies are a WARN by default (drift is signal, not a build
//! break). The orchestrator promotes WARN→FAIL when `--strict-watch` is set;
//! this adapter only reports the WARN/PASS distinction.

use crate::report::{Finding, Metric, MetricDirection, PillarResult, PillarStatus, Severity};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScoreResult {
    name: String,
    #[serde(default)]
    entity_type: String,
    cardinality: f64,
    #[serde(default)]
    flagged: bool,
    #[serde(default)]
    dimensions: Vec<DimScore>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DimScore {
    name: String,
    #[serde(default)]
    score: f64,
    #[serde(default)]
    reason: String,
}

/// Parse `cardinal-map check --json` stdout into a pillar result.
///
/// `threshold` is the configured anomaly threshold — used for the metric and
/// to recompute the flag count consistently with the engine's own flagging
/// (the engine already sets `flagged`, but a name may have been scored with a
/// different threshold; we trust the engine's `flagged` and also report how
/// many exceed the configured threshold).
pub fn parse(
    stdout: &str,
    threshold: f64,
    raw_ref: Option<String>,
) -> Result<PillarResult, serde_json::Error> {
    // `cardinal-map check --json` can emit `null` for an empty result set.
    let results: Vec<ScoreResult> = if stdout.trim().is_empty() || stdout.trim() == "null" {
        Vec::new()
    } else {
        serde_json::from_str(stdout)?
    };

    let total = results.len();
    let flagged: Vec<&ScoreResult> = results.iter().filter(|r| r.flagged).collect();
    let above_threshold = results
        .iter()
        .filter(|r| r.cardinality >= threshold)
        .count();

    let mut metrics = vec![
        Metric::plain("watch.items_scored", total as f64, MetricDirection::Neutral),
        Metric::plain(
            "watch.items_flagged",
            flagged.len() as f64,
            MetricDirection::LowerBetter,
        ),
        Metric::plain(
            "watch.items_above_threshold",
            above_threshold as f64,
            MetricDirection::LowerBetter,
        ),
        Metric::plain(
            "watch.anomaly_threshold",
            threshold,
            MetricDirection::Neutral,
        ),
    ];
    if total > 0 {
        let mean: f64 = results.iter().map(|r| r.cardinality).sum::<f64>() / total as f64;
        let max = results
            .iter()
            .map(|r| r.cardinality)
            .fold(0.0_f64, f64::max);
        metrics.push(Metric::plain(
            "watch.mean_cardinality",
            mean,
            MetricDirection::LowerBetter,
        ));
        metrics.push(Metric::plain(
            "watch.max_cardinality",
            max,
            MetricDirection::LowerBetter,
        ));
    }

    let mut findings: Vec<Finding> = Vec::new();
    for r in &flagged {
        // The top driving dimension is the most useful "why".
        let why = r
            .dimensions
            .iter()
            .filter(|d| d.name != "_source")
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|d| {
                if d.reason.is_empty() {
                    format!("driven by `{}` (score {:.2})", d.name, d.score)
                } else {
                    format!("{} (`{}` score {:.2})", d.reason, d.name, d.score)
                }
            })
            .unwrap_or_else(|| "no dimension detail".to_string());
        let entity = if r.entity_type.is_empty() {
            "item".to_string()
        } else {
            r.entity_type.clone()
        };
        findings.push(
            Finding::new(
                Severity::Major,
                format!("anomalous {}: {}", entity, r.name),
                format!("cardinality {:.3} — {}", r.cardinality, why),
            )
            .with_location(r.name.clone()),
        );
    }

    let status = if flagged.is_empty() {
        PillarStatus::Pass
    } else {
        PillarStatus::Warn
    };

    let headline = match status {
        PillarStatus::Pass => format!(
            "No drift: all {} monitored item(s) scored below the anomaly threshold.",
            total
        ),
        PillarStatus::Warn => format!(
            "{} of {} monitored item(s) look anomalous — review before trusting the output.",
            flagged.len(),
            total
        ),
        _ => "Watch ran.".to_string(),
    };

    let mut result = PillarResult::new(status, headline);
    result.metrics = metrics;
    result.findings = findings;
    result.raw_ref = raw_ref;
    Ok(result)
}

/// The install hint shown when the `cardinal-map` binary is absent.
pub fn install_hint() -> &'static str {
    "build from the cardinal-map repo (`go build`) and set [watch].bin to it"
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: &str = include_str!("../../tests/fixtures/cardinal_clean.json");
    const ANOMALY: &str = include_str!("../../tests/fixtures/cardinal_anomaly.json");

    #[test]
    fn happy_parse_no_anomalies_is_pass() {
        let r = parse(CLEAN, 0.6, None).unwrap();
        assert_eq!(r.status, PillarStatus::Pass);
        let scored = r
            .metrics
            .iter()
            .find(|m| m.name == "watch.items_scored")
            .unwrap();
        assert_eq!(scored.value, 3.0);
        assert!(r.headline.contains("No drift"));
    }

    #[test]
    fn flagged_items_warn_and_become_findings() {
        let r = parse(ANOMALY, 0.6, None).unwrap();
        assert_eq!(r.status, PillarStatus::Warn);
        assert!(r.findings.iter().any(|f| f.title.contains("Klingon")));
        let f = r
            .findings
            .iter()
            .find(|f| f.title.contains("Klingon"))
            .unwrap();
        // Detail carries the driving dimension reason.
        assert!(f.detail.contains("cardinality"));
    }

    #[test]
    fn null_output_is_empty_pass() {
        let r = parse("null", 0.6, None).unwrap();
        assert_eq!(r.status, PillarStatus::Pass);
        assert_eq!(
            r.metrics
                .iter()
                .find(|m| m.name == "watch.items_scored")
                .unwrap()
                .value,
            0.0
        );
    }

    #[test]
    fn malformed_json_errors() {
        assert!(parse("{not an array", 0.6, None).is_err());
    }
}
