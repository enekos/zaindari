//! Orchestrator — invoke the configured engines and assemble a
//! [`ZaindariReport`]. The *parsing* lives in the adapters and is pure; this
//! module owns the side-effecting invocation and the raw-output capture.

use crate::adapters::{gate, guard, native, watch};
use crate::config::{Config, GateConfig, GuardConfig, WatchConfig};
use crate::engine::{self, EngineError};
use crate::report::{PillarResult, PillarStatus, ZaindariReport};
use std::path::{Path, PathBuf};

/// Which pillars to run this invocation.
#[derive(Debug, Clone, Copy)]
pub struct Selection {
    pub gate: bool,
    pub guard: bool,
    pub watch: bool,
}

impl Selection {
    /// Every pillar.
    pub fn all() -> Self {
        Self {
            gate: true,
            guard: true,
            watch: true,
        }
    }
    /// Just one pillar.
    pub fn only_gate() -> Self {
        Self {
            gate: true,
            guard: false,
            watch: false,
        }
    }
    pub fn only_guard() -> Self {
        Self {
            gate: false,
            guard: true,
            watch: false,
        }
    }
    pub fn only_watch() -> Self {
        Self {
            gate: false,
            guard: false,
            watch: true,
        }
    }
}

/// Where to write captured raw engine output, if anywhere.
#[derive(Debug, Clone)]
pub struct RunContext<'a> {
    /// Working directory engines are invoked from (where relative config
    /// paths resolve). Usually the directory holding `zaindari.toml`.
    pub cwd: &'a Path,
    /// Directory raw engine output is written into. `None` = don't persist
    /// raw output (parsing still happens in-memory).
    pub raw_dir: Option<PathBuf>,
}

/// Run the selected, configured pillars and assemble the report.
///
/// A configured-but-unselected pillar is reported `Skipped`. An unconfigured
/// pillar is left `None`. A missing engine binary yields `engine_missing` —
/// never an error return.
pub fn run(cfg: &Config, sel: Selection, ctx: &RunContext) -> ZaindariReport {
    let mut report = ZaindariReport::empty();

    if let Some(gc) = &cfg.gate {
        report.pillars.gate = Some(if sel.gate {
            run_gate(gc, ctx, &mut report.tool_versions.gate)
        } else {
            PillarResult::new(PillarStatus::Skipped, "gate not selected this run")
        });
    }
    if let Some(gc) = &cfg.guard {
        report.pillars.guard = Some(if sel.guard {
            run_guard(gc, ctx)
        } else {
            PillarResult::new(PillarStatus::Skipped, "guard not selected this run")
        });
    }
    if let Some(wc) = &cfg.watch {
        report.pillars.watch = Some(if sel.watch {
            run_watch(wc, ctx)
        } else {
            PillarResult::new(PillarStatus::Skipped, "watch not selected this run")
        });
    }

    report
}

fn run_gate(gc: &GateConfig, ctx: &RunContext, version_slot: &mut Option<String>) -> PillarResult {
    // Native-emitter mode: the consumer's own harness writes the envelope.
    if let Some(cmd) = &gc.report_cmd {
        return run_native(cmd, ctx, "gate-native.json", version_slot);
    }

    let out_path = raw_path(ctx, "gate-aatxe-evals.json");
    let mut args: Vec<String> = vec!["evals".to_string()];
    let out_for_engine = out_path
        .clone()
        .unwrap_or_else(|| ctx.cwd.join("zaindari-aatxe-evals.json"));
    args.push("--out".to_string());
    args.push(out_for_engine.to_string_lossy().into_owned());
    if let Some(corpus) = &gc.corpus {
        args.push("--corpus".to_string());
        args.push(corpus.to_string_lossy().into_owned());
    }
    if let Some(baseline) = &gc.baseline {
        args.push("--baseline".to_string());
        args.push(baseline.to_string_lossy().into_owned());
    }
    args.extend(gc.flags.iter().cloned());

    let run = match engine::run(&gc.bin, &args, ctx.cwd) {
        Ok(r) => r,
        Err(EngineError::Missing(_)) => {
            return PillarResult::engine_missing(&gc.bin, gate::install_hint())
        }
        Err(e) => return PillarResult::new(PillarStatus::EngineMissing, e.to_string()),
    };

    // aatxe writes the report to --out; read it back.
    let current_json = match std::fs::read_to_string(&out_for_engine) {
        Ok(s) => s,
        Err(e) => {
            return PillarResult::new(
                PillarStatus::EngineMissing,
                format!(
                    "aatxe ran (exit {:?}) but its eval JSON at {} was unreadable: {e}",
                    run.exit_code,
                    out_for_engine.display()
                ),
            )
        }
    };
    let baseline_json = gc
        .baseline
        .as_ref()
        .and_then(|p| std::fs::read_to_string(ctx.cwd.join(p)).ok());

    *version_slot = extract_aatxe_version(&current_json);

    match gate::parse(
        &current_json,
        baseline_json.as_deref(),
        run.exit_code,
        out_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
    ) {
        Ok(r) => r,
        Err(e) => PillarResult::new(
            PillarStatus::EngineMissing,
            format!("aatxe eval JSON did not parse: {e}"),
        ),
    }
}

fn extract_aatxe_version(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.get("aatxeVersion")
        .and_then(|x| x.as_str())
        .map(str::to_string)
}

/// Drive a pillar from an arbitrary native-emitter command. zaindari picks the
/// output path, substitutes the `{out}` token in the command's args, runs it,
/// then reads back the native envelope. The emitter owns pass/fail via
/// `pillar.status`; a non-zero exit is fine as long as the envelope was written
/// (e.g. an eval harness that exits 1 on regression still reports its FAIL).
fn run_native(
    cmd: &[String],
    ctx: &RunContext,
    raw_name: &str,
    version_slot: &mut Option<String>,
) -> PillarResult {
    let Some((bin, rest)) = cmd.split_first() else {
        return PillarResult::new(
            PillarStatus::EngineMissing,
            "report_cmd is empty — give it `[program, args…]`",
        );
    };

    let raw = raw_path(ctx, raw_name);
    let out_for_engine = raw
        .clone()
        .unwrap_or_else(|| ctx.cwd.join(format!("zaindari-{raw_name}")));
    let out_str = out_for_engine.to_string_lossy();
    let args: Vec<String> = rest.iter().map(|a| a.replace("{out}", &out_str)).collect();

    let run = match engine::run(bin, &args, ctx.cwd) {
        Ok(r) => r,
        Err(EngineError::Missing(_)) => {
            return PillarResult::engine_missing(bin, native::install_hint())
        }
        Err(e) => return PillarResult::new(PillarStatus::EngineMissing, e.to_string()),
    };

    let json = match std::fs::read_to_string(&out_for_engine) {
        Ok(s) => s,
        Err(e) => {
            return PillarResult::new(
                PillarStatus::EngineMissing,
                format!(
                    "`{bin}` ran (exit {:?}) but wrote no native report at {}: {e}{}",
                    run.exit_code,
                    out_for_engine.display(),
                    stderr_tail(&run.stderr),
                ),
            )
        }
    };

    match native::parse(&json, raw.map(|p| p.to_string_lossy().into_owned())) {
        Ok((pillar, ver)) => {
            if ver.is_some() {
                *version_slot = ver;
            }
            pillar
        }
        Err(e) => PillarResult::new(PillarStatus::EngineMissing, e.to_string()),
    }
}

/// A trailing snippet of stderr for error context, prefixed only when present.
fn stderr_tail(stderr: &str) -> String {
    let t = stderr.trim();
    if t.is_empty() {
        String::new()
    } else {
        format!(" — stderr: {t}")
    }
}

fn run_guard(gc: &GuardConfig, ctx: &RunContext) -> PillarResult {
    let mut args: Vec<String> = vec!["test".to_string(), "--json".to_string()];
    for p in &gc.packs {
        args.push(p.to_string_lossy().into_owned());
    }
    let run = match engine::run(&gc.bin, &args, ctx.cwd) {
        Ok(r) => r,
        Err(EngineError::Missing(_)) => {
            return PillarResult::engine_missing(&gc.bin, guard::install_hint())
        }
        Err(e) => return PillarResult::new(PillarStatus::EngineMissing, e.to_string()),
    };
    let raw = raw_path(ctx, "guard-iratxo-test.json");
    if let Some(p) = &raw {
        let _ = std::fs::write(p, &run.stdout);
    }
    match guard::parse(
        &run.stdout,
        run.exit_code,
        raw.map(|p| p.to_string_lossy().into_owned()),
    ) {
        Ok(r) => r,
        Err(e) => PillarResult::new(
            PillarStatus::EngineMissing,
            format!("iratxo test --json output did not parse: {e}"),
        ),
    }
}

fn run_watch(wc: &WatchConfig, ctx: &RunContext) -> PillarResult {
    let args: Vec<String> = vec![
        "check".to_string(),
        "--json".to_string(),
        "--schema".to_string(),
        wc.schema.to_string_lossy().into_owned(),
        "--profiles".to_string(),
        wc.profiles.to_string_lossy().into_owned(),
        "--input".to_string(),
        wc.input.to_string_lossy().into_owned(),
        "--threshold".to_string(),
        wc.anomaly_threshold.to_string(),
    ];
    let run = match engine::run(&wc.bin, &args, ctx.cwd) {
        Ok(r) => r,
        Err(EngineError::Missing(_)) => {
            return PillarResult::engine_missing(&wc.bin, watch::install_hint())
        }
        Err(e) => return PillarResult::new(PillarStatus::EngineMissing, e.to_string()),
    };
    // Non-zero exit from cardinal-map is an operational error (bad schema /
    // unreadable input), not an anomaly.
    if run.exit_code != Some(0) {
        return PillarResult::new(
            PillarStatus::EngineMissing,
            format!(
                "cardinal-map check failed (exit {:?}): {}",
                run.exit_code,
                run.stderr.trim()
            ),
        );
    }
    let raw = raw_path(ctx, "watch-cardinal-check.json");
    if let Some(p) = &raw {
        let _ = std::fs::write(p, &run.stdout);
    }
    match watch::parse(
        &run.stdout,
        wc.anomaly_threshold,
        raw.map(|p| p.to_string_lossy().into_owned()),
    ) {
        Ok(r) => r,
        Err(e) => PillarResult::new(
            PillarStatus::EngineMissing,
            format!("cardinal-map JSON did not parse: {e}"),
        ),
    }
}

fn raw_path(ctx: &RunContext, name: &str) -> Option<PathBuf> {
    ctx.raw_dir.as_ref().map(|d| d.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::PillarStatus;

    #[test]
    fn unconfigured_pillars_stay_none() {
        let cfg = Config::default();
        let ctx = RunContext {
            cwd: Path::new("."),
            raw_dir: None,
        };
        let report = run(&cfg, Selection::all(), &ctx);
        assert!(report.pillars.gate.is_none());
        assert!(report.pillars.guard.is_none());
        assert!(report.pillars.watch.is_none());
    }

    #[test]
    fn configured_but_unselected_pillar_is_skipped() {
        let cfg = Config {
            guard: Some(GuardConfig {
                bin: "iratxo".into(),
                packs: vec!["x.cases.yaml".into()],
            }),
            ..Default::default()
        };
        let ctx = RunContext {
            cwd: Path::new("."),
            raw_dir: None,
        };
        let report = run(&cfg, Selection::only_gate(), &ctx);
        let g = report.pillars.guard.unwrap();
        assert_eq!(g.status, PillarStatus::Skipped);
    }

    #[test]
    fn missing_engine_binary_yields_engine_missing_not_panic() {
        let cfg = Config {
            guard: Some(GuardConfig {
                bin: "zaindari-no-such-binary-xyz".into(),
                packs: vec!["x.cases.yaml".into()],
            }),
            ..Default::default()
        };
        let ctx = RunContext {
            cwd: Path::new("."),
            raw_dir: None,
        };
        let report = run(&cfg, Selection::only_guard(), &ctx);
        let g = report.pillars.guard.unwrap();
        assert_eq!(g.status, PillarStatus::EngineMissing);
        assert!(g.headline.contains("not found"));
    }
}
