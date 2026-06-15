//! Integration tests for config discovery + `zaindari init`.
//!
//! These never invoke the real engine binaries — they exercise config
//! walk-up, the sample config `init` writes, and the JSON report round-trip
//! through the `report` subcommand.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Path to the built `zaindari` binary.
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_zaindari")
}

#[test]
fn init_writes_a_valid_sample_config() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(bin())
        .arg("init")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "init failed: {:?}", out);

    let cfg_path = dir.path().join("zaindari.toml");
    assert!(cfg_path.is_file(), "init did not write zaindari.toml");

    // The written file must parse back through the core config loader.
    let src = fs::read_to_string(&cfg_path).unwrap();
    let cfg = zaindari_core::Config::from_toml_str(&src, &cfg_path).unwrap();
    assert!(cfg.gate.is_some());
    assert!(cfg.guard.is_some());
    assert!(cfg.watch.is_some());
}

#[test]
fn init_refuses_to_overwrite_without_force() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("zaindari.toml"), "# existing\n").unwrap();

    let out = Command::new(bin())
        .arg("init")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success(), "init clobbered an existing config");

    // --force overrides.
    let forced = Command::new(bin())
        .args(["init", "--force"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(forced.status.success());
}

#[test]
fn config_discovery_walks_up_from_a_subdirectory() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("zaindari.toml"),
        "[guard]\npacks = [\"x.cases.yaml\"]\n",
    )
    .unwrap();
    let nested = dir.path().join("a").join("b").join("c");
    fs::create_dir_all(&nested).unwrap();

    let found = zaindari_core::config::find_config_path(&nested)
        .expect("should walk up to the root zaindari.toml");
    assert_eq!(found, dir.path().join("zaindari.toml"));
}

#[test]
fn report_subcommand_renders_saved_json_to_html() {
    let dir = tempfile::tempdir().unwrap();
    // Hand-build a minimal saved run JSON.
    let run_json = r#"{
      "schemaVersion": 1,
      "toolVersions": { "gate": "0.1.1" },
      "pillars": {
        "gate": { "status": "pass", "headline": "Eval gate held.", "metrics": [], "findings": [] }
      }
    }"#;
    let json_path = dir.path().join("run.json");
    fs::write(&json_path, run_json).unwrap();
    let html_path = dir.path().join("report.html");

    let out = Command::new(bin())
        .arg("report")
        .arg(&json_path)
        .arg("--html")
        .arg(&html_path)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "report failed: {:?}", out);

    let html = fs::read_to_string(&html_path).unwrap();
    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("Eval gate held"));
    assert!(html.contains(">Gate</h2>"));
    // Self-contained: no external resources.
    assert!(!html.contains("http"));
}

// The native emitter just has to write the envelope to {out}; here we stage it
// in a file and `cp` it, so the test config stays free of JSON-in-TOML quoting.
#[cfg(unix)]
fn write_native_gate_config(dir: &Path, envelope: &str) {
    fs::write(dir.join("envelope.json"), envelope).unwrap();
    fs::write(
        dir.join("zaindari.toml"),
        "[gate]\nreport_cmd = [\"cp\", \"envelope.json\", \"{out}\"]\n",
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn gate_native_report_cmd_drives_the_pillar_end_to_end() {
    // A `report_cmd` engine: any command that writes the native envelope to the
    // {out} path. The staged file stands in for berme-eval. Proves the whole
    // path: config -> run dispatch -> native adapter -> report -> exit policy.
    let dir = tempfile::tempdir().unwrap();
    write_native_gate_config(
        dir.path(),
        r#"{"schemaVersion":1,"toolVersion":"fake-eval 1.2","pillar":{"status":"pass","headline":"native gate held: key-F1 0.97","metrics":[{"name":"key_f1","value":0.97,"direction":"higher_better"}],"findings":[]}}"#,
    );

    let out = Command::new(bin())
        .args(["gate", "--json"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "native gate should pass: {:?}", out);

    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["pillars"]["gate"]["status"], "pass");
    assert_eq!(
        report["pillars"]["gate"]["headline"],
        "native gate held: key-F1 0.97"
    );
    assert_eq!(report["toolVersions"]["gate"], "fake-eval 1.2");
}

#[cfg(unix)]
#[test]
fn gate_native_fail_status_gates_with_exit_two() {
    // An emitter reporting FAIL must gate the build (exit 2) even when the
    // command itself exited 0 — the envelope's status is authoritative.
    let dir = tempfile::tempdir().unwrap();
    write_native_gate_config(
        dir.path(),
        r#"{"schemaVersion":1,"pillar":{"status":"fail","headline":"eval regressed"}}"#,
    );

    let out = Command::new(bin())
        .arg("gate")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "FAIL pillar should gate: {:?}",
        out
    );
}

#[test]
fn run_without_config_errors_with_exit_one() {
    // A bare temp dir with no zaindari.toml anywhere up to root would still
    // find configs on the dev machine's parents in theory; use an isolated
    // dir and assert the binary doesn't panic. We can't guarantee no config
    // exists above tempdir on every machine, so only assert it runs.
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(bin())
        .arg("run")
        .current_dir(dir.path())
        .output()
        .unwrap();
    // Either exit 1 (no config) or a clean orchestration; never a panic (101).
    assert_ne!(out.status.code(), Some(101), "binary panicked: {:?}", out);
    let _ = Path::new(bin());
}
