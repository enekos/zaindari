//! Guard adapter — wraps `iratxo test`.
//!
//! Engine contract (observed in iratxo @ crates/iratxo-cli/src/testing.rs):
//!   * `iratxo test <pack|suite|dir>...`
//!   * HUMAN TEXT ONLY — no JSON output today. We parse stdout + exit code.
//!   * stdout per case: `  ok   - <name>` / `  FAIL - <name>` (+ indented
//!     reason lines), and a summary line `N passed, M failed across K suite(s)`.
//!   * unrunnable suites print to stderr `  - <path>: <reason>`.
//!   * exit code: non-zero (1) if any case fails OR any suite can't resolve.
//!
//! FOLLOW-UP ASK: iratxo has no `--json`. We parse a stable-but-human format;
//! a machine-readable mode would make this adapter robust. Until then the exit
//! code is the source of truth for pass/fail and the text drives the detail.

use crate::report::{Finding, Metric, MetricDirection, PillarResult, PillarStatus, Severity};

/// Parse `iratxo test` stdout + stderr + exit code into a pillar result.
///
/// Pass/fail is keyed off the exit code (authoritative); the text is parsed
/// for counts and per-case failures. If the exit code is unknown we fall back
/// to the parsed failure count.
pub fn parse(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    raw_ref: Option<String>,
) -> PillarResult {
    let summary = parse_summary(stdout);
    let failures = parse_failures(stdout);
    let broken = parse_broken_suites(stderr);

    let failed_count = summary.map(|s| s.failed as usize).unwrap_or(failures.len());
    let exit_failed = match exit_code {
        Some(0) => false,
        Some(_) => true,
        None => failed_count > 0 || !broken.is_empty(),
    };

    let status = if exit_failed {
        PillarStatus::Fail
    } else {
        PillarStatus::Pass
    };

    let mut metrics = Vec::new();
    if let Some(s) = summary {
        metrics.push(Metric::plain(
            "guard.cases_passed",
            s.passed as f64,
            MetricDirection::HigherBetter,
        ));
        metrics.push(Metric::plain(
            "guard.cases_failed",
            s.failed as f64,
            MetricDirection::LowerBetter,
        ));
        metrics.push(Metric::plain(
            "guard.suites",
            s.suites as f64,
            MetricDirection::Neutral,
        ));
    }

    let mut findings: Vec<Finding> = Vec::new();
    for f in &failures {
        findings.push(
            Finding::new(
                Severity::Major,
                format!("rule case failed: {}", f.name),
                if f.reasons.is_empty() {
                    "case did not meet its expectation".to_string()
                } else {
                    f.reasons.join("; ")
                },
            )
            .with_location(f.name.clone()),
        );
    }
    for (path, reason) in &broken {
        findings.push(
            Finding::new(Severity::Critical, "suite could not run", reason.clone())
                .with_location(path.clone()),
        );
    }

    let headline = match (status, summary) {
        (PillarStatus::Pass, Some(s)) => format!(
            "All {} guard rule case(s) passed across {} suite(s).",
            s.passed, s.suites
        ),
        (PillarStatus::Pass, None) => "Guard rules passed.".to_string(),
        (PillarStatus::Fail, Some(s)) => format!(
            "{} guard rule case(s) failed across {} suite(s) — runtime rules are not safe.",
            s.failed, s.suites
        ),
        (PillarStatus::Fail, None) => {
            "Guard rules failed — runtime rules are not safe.".to_string()
        }
        _ => "Guard ran.".to_string(),
    };

    let mut result = PillarResult::new(status, headline);
    result.metrics = metrics;
    result.findings = findings;
    result.raw_ref = raw_ref;
    result
}

#[derive(Debug, Clone, Copy)]
struct Summary {
    passed: u32,
    failed: u32,
    suites: u32,
}

/// Parse the trailing `N passed, M failed across K suite(s)` line.
fn parse_summary(stdout: &str) -> Option<Summary> {
    // Search from the end — the summary is the last matching line.
    for line in stdout.lines().rev() {
        let l = line.trim();
        if l.contains(" passed, ") && l.contains(" failed across ") {
            return parse_summary_line(l);
        }
    }
    None
}

/// `12 passed, 0 failed across 3 suites` → Summary.
fn parse_summary_line(l: &str) -> Option<Summary> {
    let passed = take_leading_u32(l)?;
    let after_passed = l.split(" passed, ").nth(1)?;
    let failed = take_leading_u32(after_passed)?;
    let after_failed = after_passed.split(" failed across ").nth(1)?;
    let suites = take_leading_u32(after_failed)?;
    Some(Summary {
        passed,
        failed,
        suites,
    })
}

fn take_leading_u32(s: &str) -> Option<u32> {
    let digits: String = s
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

#[derive(Debug, Clone)]
struct CaseFailure {
    name: String,
    reasons: Vec<String>,
}

/// Parse `  FAIL - <name>` lines plus their indented reason lines.
fn parse_failures(stdout: &str) -> Vec<CaseFailure> {
    let mut out: Vec<CaseFailure> = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim_start();
        if let Some(name) = trimmed.strip_prefix("FAIL - ") {
            out.push(CaseFailure {
                name: name.trim().to_string(),
                reasons: Vec::new(),
            });
        } else if !out.is_empty()
            && line.starts_with("         ")
            && !trimmed.starts_with("ok ")
            && !trimmed.starts_with("FAIL - ")
            && !trimmed.is_empty()
        {
            // Deeply-indented continuation line = a reason for the last FAIL.
            if let Some(last) = out.last_mut() {
                last.reasons.push(trimmed.to_string());
            }
        }
    }
    out
}

/// Parse the stderr `  - <path>: <reason>` lines for unrunnable suites.
fn parse_broken_suites(stderr: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("- ") {
            if let Some((path, reason)) = rest.split_once(": ") {
                out.push((path.trim().to_string(), reason.trim().to_string()));
            }
        }
    }
    out
}

/// The install hint shown when the `iratxo` binary is absent.
pub fn install_hint() -> &'static str {
    "build from the iratxo repo (`cargo build -p iratxo-cli`) and set [guard].bin to it"
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS: &str = include_str!("../../tests/fixtures/iratxo_pass.txt");
    const FAIL: &str = include_str!("../../tests/fixtures/iratxo_fail.txt");

    #[test]
    fn happy_parse_all_passing() {
        let r = parse(PASS, "", Some(0), None);
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
        let r = parse(FAIL, "", Some(1), None);
        assert_eq!(r.status, PillarStatus::Fail);
        assert!(r
            .findings
            .iter()
            .any(|f| f.title.contains("promo-missing-disclaimer")));
        // Reason line attached.
        let f = r
            .findings
            .iter()
            .find(|f| f.title.contains("promo-missing-disclaimer"))
            .unwrap();
        assert!(f.detail.contains("triggers"));
    }

    #[test]
    fn nonzero_exit_with_no_parsed_summary_still_fails() {
        let r = parse("garbage output", "", Some(1), None);
        assert_eq!(r.status, PillarStatus::Fail);
    }

    #[test]
    fn unknown_exit_falls_back_to_parsed_failures() {
        let r = parse(FAIL, "", None, None);
        assert_eq!(r.status, PillarStatus::Fail);
    }

    #[test]
    fn broken_suite_on_stderr_is_critical_finding() {
        let stderr = "1 suite could not run:\n  - rules/bad.cases.yaml: rule pack not found\n";
        let r = parse("0 passed, 0 failed across 0 suites", stderr, Some(1), None);
        assert!(r
            .findings
            .iter()
            .any(|f| f.severity == Severity::Critical && f.title.contains("could not run")));
    }
}
