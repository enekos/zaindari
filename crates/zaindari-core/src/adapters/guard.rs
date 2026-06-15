//! Guard adapter — wraps `iratxo test --json`.
//!
//! Engine contract (observed in iratxo @ crates/iratxo-cli/src/testing.rs):
//! `iratxo test --json <pack|suite|dir>...` emits ONE JSON object on stdout:
//!
//! ```text
//! { schema_version, summary: { total, passed, failed, suites },
//!   cases: [ { name, status: "pass"|"fail", pack, suite, detail? } ],
//!   broken_suites?: [ { suite, reason } ] }
//! ```
//!
//! `detail` is the array of per-assertion failure reasons; `broken_suites`
//! lists suites that couldn't resolve. Both omitted when empty. Exit code is
//! non-zero (1) if any case fails OR any suite can't resolve.
//!
//! The exit code stays authoritative for pass/fail; the JSON drives counts,
//! per-case findings, and broken-suite findings. (iratxo gained `--json`
//! 2026-06-15 — this replaced the previous human-text scraping.)

use crate::report::{Finding, Metric, MetricDirection, PillarResult, PillarStatus, Severity};
use serde::Deserialize;

/// The `iratxo test --json` report envelope.
#[derive(Debug, Clone, Deserialize)]
struct IratxoReport {
    summary: Summary,
    #[serde(default)]
    cases: Vec<Case>,
    #[serde(default)]
    broken_suites: Vec<BrokenSuite>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct Summary {
    passed: u32,
    failed: u32,
    suites: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct Case {
    name: String,
    status: String,
    #[serde(default)]
    detail: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct BrokenSuite {
    suite: String,
    reason: String,
}

/// Parse `iratxo test --json` stdout + exit code into a pillar result.
///
/// Pass/fail is keyed off the exit code (authoritative); the JSON supplies
/// counts and per-case detail. If the exit code is unknown we fall back to the
/// parsed failed-count and broken-suite list.
pub fn parse(
    stdout: &str,
    exit_code: Option<i32>,
    raw_ref: Option<String>,
) -> Result<PillarResult, serde_json::Error> {
    let report: IratxoReport = serde_json::from_str(stdout.trim())?;
    let s = report.summary;

    let exit_failed = match exit_code {
        Some(0) => false,
        Some(_) => true,
        None => s.failed > 0 || !report.broken_suites.is_empty(),
    };
    let status = if exit_failed {
        PillarStatus::Fail
    } else {
        PillarStatus::Pass
    };

    let metrics = vec![
        Metric::plain(
            "guard.cases_passed",
            s.passed as f64,
            MetricDirection::HigherBetter,
        ),
        Metric::plain(
            "guard.cases_failed",
            s.failed as f64,
            MetricDirection::LowerBetter,
        ),
        Metric::plain("guard.suites", s.suites as f64, MetricDirection::Neutral),
    ];

    let mut findings: Vec<Finding> = Vec::new();
    for c in report.cases.iter().filter(|c| c.status != "pass") {
        findings.push(
            Finding::new(
                Severity::Major,
                format!("rule case failed: {}", c.name),
                if c.detail.is_empty() {
                    "case did not meet its expectation".to_string()
                } else {
                    c.detail.join("; ")
                },
            )
            .with_location(c.name.clone()),
        );
    }
    for b in &report.broken_suites {
        findings.push(
            Finding::new(Severity::Critical, "suite could not run", b.reason.clone())
                .with_location(b.suite.clone()),
        );
    }

    let headline = match status {
        PillarStatus::Pass => format!(
            "All {} guard rule case(s) passed across {} suite(s).",
            s.passed, s.suites
        ),
        _ => format!(
            "{} guard rule case(s) failed across {} suite(s) — runtime rules are not safe.",
            s.failed, s.suites
        ),
    };

    let mut result = PillarResult::new(status, headline);
    result.metrics = metrics;
    result.findings = findings;
    result.raw_ref = raw_ref;
    Ok(result)
}

/// The install hint shown when the `iratxo` binary is absent.
pub fn install_hint() -> &'static str {
    "build from the iratxo repo (`cargo build -p iratxo-cli`) and set [guard].bin to it"
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS: &str = include_str!("../../tests/fixtures/iratxo_pass.json");
    const FAIL: &str = include_str!("../../tests/fixtures/iratxo_fail.json");

    #[test]
    fn happy_parse_all_passing() {
        let r = parse(PASS, Some(0), None).unwrap();
        assert_eq!(r.status, PillarStatus::Pass);
        let passed = r
            .metrics
            .iter()
            .find(|m| m.name == "guard.cases_passed")
            .unwrap();
        assert_eq!(passed.value, 108.0);
        assert!(r.headline.contains("passed"));
    }

    #[test]
    fn failing_cases_become_findings_and_fail_status() {
        let r = parse(FAIL, Some(1), None).unwrap();
        assert_eq!(r.status, PillarStatus::Fail);
        assert!(r
            .findings
            .iter()
            .any(|f| f.title.contains("promo-missing-disclaimer")));
        // Detail line attached.
        let f = r
            .findings
            .iter()
            .find(|f| f.title.contains("promo-missing-disclaimer"))
            .unwrap();
        assert!(f.detail.contains("triggers"));
    }

    #[test]
    fn nonzero_exit_overrides_clean_summary() {
        // Exit code is authoritative: a non-zero exit fails even if the JSON
        // summary shows zero failures (e.g. a partial-write / engine fault).
        let r = parse(PASS, Some(1), None).unwrap();
        assert_eq!(r.status, PillarStatus::Fail);
    }

    #[test]
    fn unknown_exit_falls_back_to_parsed_failures() {
        let r = parse(FAIL, None, None).unwrap();
        assert_eq!(r.status, PillarStatus::Fail);
    }

    #[test]
    fn broken_suite_is_critical_finding() {
        let json = r#"{"schema_version":1,"summary":{"total":0,"passed":0,"failed":0,"suites":0},"cases":[],"broken_suites":[{"suite":"rules/bad.cases.yaml","reason":"rule pack not found"}]}"#;
        let r = parse(json, Some(1), None).unwrap();
        assert!(r
            .findings
            .iter()
            .any(|f| f.severity == Severity::Critical && f.title.contains("could not run")));
    }

    #[test]
    fn non_json_stdout_is_an_error() {
        assert!(parse("garbage output", Some(1), None).is_err());
    }
}
