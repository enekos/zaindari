//! Gate adapter — wraps `aatxe evals`.
//!
//! Engine contract (observed in aatxe @ crates/aatxe-evals + commands/evals.rs):
//!   * `aatxe evals --out <json> [--baseline <json>] [--corpus <dir>] <flags>`
//!   * writes a JSON `EvalReport` to `--out`
//!   * exit 0 = ok, exit 2 = regression vs baseline past tolerance, exit 1 = error
//!
//! The eval JSON is camelCase. We deserialize the subset we surface and map
//! the council headline metrics + a regression count into the shared model.
//! We key pass/fail off the JSON content (regression in any baselined metric)
//! AND off the exit code when available, so the gate is correct whether the
//! caller hands us the process exit code or only the JSON file.

use crate::report::{Finding, Metric, MetricDirection, PillarResult, PillarStatus, Severity};
use serde::Deserialize;

/// Subset of aatxe's on-disk `EvalReport` we care about.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AatxeEvalReport {
    #[serde(default)]
    aatxe_version: String,
    #[serde(default)]
    council: Option<Council>,
    #[serde(default)]
    stats: Option<Stats>,
    #[serde(default)]
    council_used_real_llm: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Council {
    cases_total: u32,
    cases_fully_recalled: u32,
    critical_recall: f64,
    critical_precision: f64,
    critical_f1: f64,
    severity_calibration_mae: f64,
    judge_brier_score: f64,
    avg_false_positives_per_case: f64,
    forbidden_path_findings: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stats {
    scenarios_total: u32,
    scenarios_passed: u32,
    pass_rate: f64,
}

/// Parse an aatxe eval JSON plus the optional process exit code into a pillar
/// result. A baseline report (same shape) enables baseline deltas.
///
/// Pass/fail policy: FAIL if the exit code was 2 (aatxe's regression code) OR,
/// when a baseline is given, if any headline council metric regressed past a
/// nominal band. Otherwise PASS.
pub fn parse(
    current_json: &str,
    baseline_json: Option<&str>,
    exit_code: Option<i32>,
    raw_ref: Option<String>,
) -> Result<PillarResult, serde_json::Error> {
    let cur: AatxeEvalReport = serde_json::from_str(current_json)?;
    let base: Option<AatxeEvalReport> = match baseline_json {
        Some(s) => Some(serde_json::from_str(s)?),
        None => None,
    };

    let mut metrics: Vec<Metric> = Vec::new();
    let mut findings: Vec<Finding> = Vec::new();

    let base_council = base.as_ref().and_then(|b| b.council.as_ref());

    if let Some(c) = &cur.council {
        push_council_metrics(&mut metrics, c, base_council);
        if c.forbidden_path_findings > 0 {
            findings.push(Finding::new(
                Severity::Critical,
                "findings on forbidden paths",
                format!(
                    "{} finding(s) landed on lockfiles / generated code — calibration bug",
                    c.forbidden_path_findings
                ),
            ));
        }
    }
    if let Some(s) = &cur.stats {
        let base_stats = base.as_ref().and_then(|b| b.stats.as_ref());
        match base_stats {
            Some(bs) => metrics.push(Metric::with_baseline(
                "stats.pass_rate",
                s.pass_rate,
                bs.pass_rate,
                MetricDirection::HigherBetter,
            )),
            None => metrics.push(Metric::plain(
                "stats.pass_rate",
                s.pass_rate,
                MetricDirection::HigherBetter,
            )),
        }
        if s.scenarios_passed < s.scenarios_total {
            findings.push(Finding::new(
                Severity::Major,
                "stats scenarios failed",
                format!(
                    "{}/{} stats scenarios passed",
                    s.scenarios_passed, s.scenarios_total
                ),
            ));
        }
    }

    // A regression is: exit code 2, or any baselined metric that moved the
    // wrong way. The exit code is authoritative when present (aatxe already
    // applied its tolerance bands); the metric check is the fallback when we
    // only have JSON files.
    let metric_regressed = metrics.iter().any(Metric::regressed);
    let regressed = exit_code == Some(2) || (baseline_json.is_some() && metric_regressed);

    let status = if regressed {
        PillarStatus::Fail
    } else {
        PillarStatus::Pass
    };

    let headline = build_headline(&cur, status, baseline_json.is_some());

    let mut result = PillarResult::new(status, headline);
    result.metrics = metrics;
    result.findings = findings;
    result.raw_ref = raw_ref;
    Ok(result)
}

fn push_council_metrics(metrics: &mut Vec<Metric>, c: &Council, base: Option<&Council>) {
    // (name, current getter, direction)
    type Get = fn(&Council) -> f64;
    let rows: &[(&str, Get, MetricDirection)] = &[
        (
            "council.critical_recall",
            |c| c.critical_recall,
            MetricDirection::HigherBetter,
        ),
        (
            "council.critical_precision",
            |c| c.critical_precision,
            MetricDirection::HigherBetter,
        ),
        (
            "council.critical_f1",
            |c| c.critical_f1,
            MetricDirection::HigherBetter,
        ),
        (
            "council.severity_calibration_mae",
            |c| c.severity_calibration_mae,
            MetricDirection::LowerBetter,
        ),
        (
            "council.judge_brier_score",
            |c| c.judge_brier_score,
            MetricDirection::LowerBetter,
        ),
        (
            "council.avg_false_positives_per_case",
            |c| c.avg_false_positives_per_case,
            MetricDirection::LowerBetter,
        ),
    ];
    for (name, get, dir) in rows {
        let v = get(c);
        match base {
            Some(b) => metrics.push(Metric::with_baseline(*name, v, get(b), *dir)),
            None => metrics.push(Metric::plain(*name, v, *dir)),
        }
    }
    metrics.push(Metric::plain(
        "council.cases_fully_recalled",
        c.cases_fully_recalled as f64,
        MetricDirection::HigherBetter,
    ));
    metrics.push(Metric::plain(
        "council.cases_total",
        c.cases_total as f64,
        MetricDirection::Neutral,
    ));
    metrics.push(Metric::plain(
        "council.forbidden_path_findings",
        c.forbidden_path_findings as f64,
        MetricDirection::LowerBetter,
    ));
}

fn build_headline(cur: &AatxeEvalReport, status: PillarStatus, had_baseline: bool) -> String {
    let llm = if cur.council_used_real_llm {
        "real-LLM"
    } else {
        "stub-LLM"
    };
    match (status, cur.council.as_ref()) {
        (PillarStatus::Fail, _) if had_baseline => {
            "Eval quality regressed against the baseline — do not ship.".to_string()
        }
        (PillarStatus::Fail, _) => "Eval gate failed — do not ship.".to_string(),
        (_, Some(c)) => format!(
            "Eval gate held: critical F1 {:.2}, {}/{} cases fully recalled ({} run, aatxe {}).",
            c.critical_f1, c.cases_fully_recalled, c.cases_total, llm, cur.aatxe_version
        ),
        (_, None) => format!("Eval gate held ({} run, aatxe {}).", llm, cur.aatxe_version),
    }
}

/// The install hint shown when the `aatxe` binary is absent.
pub fn install_hint() -> &'static str {
    "install with `cargo install aatxe`, or set [gate].bin to its path"
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = include_str!("../../tests/fixtures/aatxe_good.json");
    const REGRESSED: &str = include_str!("../../tests/fixtures/aatxe_regressed.json");
    const BASELINE: &str = include_str!("../../tests/fixtures/aatxe_baseline.json");

    #[test]
    fn happy_parse_no_baseline_is_pass() {
        let r = parse(GOOD, None, Some(0), None).unwrap();
        assert_eq!(r.status, PillarStatus::Pass);
        assert!(r.metrics.iter().any(|m| m.name == "council.critical_f1"));
        assert!(r.headline.contains("Eval gate held"));
    }

    #[test]
    fn exit_code_2_forces_fail_even_without_baseline() {
        let r = parse(GOOD, None, Some(2), None).unwrap();
        assert_eq!(r.status, PillarStatus::Fail);
    }

    #[test]
    fn baseline_regression_in_metric_is_fail() {
        // Exit code unknown (None) — must still catch the drop from the JSON.
        let r = parse(REGRESSED, Some(BASELINE), None, None).unwrap();
        assert_eq!(r.status, PillarStatus::Fail);
        let f1 = r
            .metrics
            .iter()
            .find(|m| m.name == "council.critical_f1")
            .unwrap();
        assert!(f1.baseline.is_some());
        assert!(f1.regressed());
    }

    #[test]
    fn baseline_with_improvement_is_pass() {
        let r = parse(GOOD, Some(BASELINE), Some(0), None).unwrap();
        assert_eq!(r.status, PillarStatus::Pass);
    }

    #[test]
    fn forbidden_path_finding_surfaces_as_critical() {
        let r = parse(REGRESSED, Some(BASELINE), Some(2), None).unwrap();
        assert!(r
            .findings
            .iter()
            .any(|f| f.severity == Severity::Critical && f.title.contains("forbidden")));
    }

    #[test]
    fn malformed_json_errors() {
        assert!(parse("{not json", None, Some(0), None).is_err());
    }
}
