//! Exit-code policy — the one place that turns a [`ZaindariReport`] into a
//! process exit code, so the CLI binary stays a thin wrapper.
//!
//! Policy (CI-gate semantics):
//!   * gate or guard FAIL → exit 2 (block the build)
//!   * watch anomalies (WARN) → exit 0 by default; exit 2 if `strict_watch`
//!   * any engine error / engine_missing → exit 1
//!   * everything pass/skipped → exit 0

use crate::report::{PillarResult, PillarStatus, Pillars, ZaindariReport};

/// Knobs that change how statuses map to codes.
#[derive(Debug, Clone, Copy, Default)]
pub struct PolicyOptions {
    /// Promote watch anomalies (WARN) to a gating failure.
    pub strict_watch: bool,
    /// Treat a missing engine binary as a hard error (exit 1). When false a
    /// missing engine still yields exit 1 — engine_missing is always an error
    /// — but this flag is reserved for future "skip missing" behaviour.
    pub error_on_missing: bool,
}

/// The exit code zaindari should return for a finished report.
pub fn exit_code(report: &ZaindariReport, opts: PolicyOptions) -> i32 {
    let p = &report.pillars;

    // 1 — any engine that couldn't run (missing binary) is an error.
    if any(p, |r| r.status == PillarStatus::EngineMissing) {
        return 1;
    }

    // 2 — gate/guard failure gates the build.
    let gate_failed = is(p.gate.as_ref(), PillarStatus::Fail);
    let guard_failed = is(p.guard.as_ref(), PillarStatus::Fail);
    if gate_failed || guard_failed {
        return 2;
    }

    // strict-watch: a watch WARN becomes a gating failure.
    if opts.strict_watch && is(p.watch.as_ref(), PillarStatus::Warn) {
        return 2;
    }

    0
}

fn is(r: Option<&PillarResult>, status: PillarStatus) -> bool {
    r.map(|x| x.status == status).unwrap_or(false)
}

fn any(p: &Pillars, pred: impl Fn(&PillarResult) -> bool) -> bool {
    [p.gate.as_ref(), p.guard.as_ref(), p.watch.as_ref()]
        .into_iter()
        .flatten()
        .any(pred)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::PillarResult;

    fn report_with(
        gate: Option<PillarStatus>,
        guard: Option<PillarStatus>,
        watch: Option<PillarStatus>,
    ) -> ZaindariReport {
        let mut r = ZaindariReport::empty();
        r.pillars.gate = gate.map(|s| PillarResult::new(s, "g"));
        r.pillars.guard = guard.map(|s| PillarResult::new(s, "g"));
        r.pillars.watch = watch.map(|s| PillarResult::new(s, "w"));
        r
    }

    #[test]
    fn all_pass_is_zero() {
        let r = report_with(Some(PillarStatus::Pass), Some(PillarStatus::Pass), None);
        assert_eq!(exit_code(&r, PolicyOptions::default()), 0);
    }

    #[test]
    fn gate_fail_is_two() {
        let r = report_with(Some(PillarStatus::Fail), None, None);
        assert_eq!(exit_code(&r, PolicyOptions::default()), 2);
    }

    #[test]
    fn guard_fail_is_two() {
        let r = report_with(None, Some(PillarStatus::Fail), None);
        assert_eq!(exit_code(&r, PolicyOptions::default()), 2);
    }

    #[test]
    fn watch_warn_is_zero_by_default() {
        let r = report_with(None, None, Some(PillarStatus::Warn));
        assert_eq!(exit_code(&r, PolicyOptions::default()), 0);
    }

    #[test]
    fn watch_warn_is_two_under_strict() {
        let r = report_with(None, None, Some(PillarStatus::Warn));
        let opts = PolicyOptions {
            strict_watch: true,
            ..Default::default()
        };
        assert_eq!(exit_code(&r, opts), 2);
    }

    #[test]
    fn engine_missing_is_one_and_beats_a_failure() {
        let r = report_with(
            Some(PillarStatus::Fail),
            Some(PillarStatus::EngineMissing),
            None,
        );
        assert_eq!(exit_code(&r, PolicyOptions::default()), 1);
    }

    #[test]
    fn skipped_pillars_do_not_gate() {
        let r = report_with(
            Some(PillarStatus::Skipped),
            Some(PillarStatus::Skipped),
            Some(PillarStatus::Skipped),
        );
        assert_eq!(exit_code(&r, PolicyOptions::default()), 0);
    }
}
