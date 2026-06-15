//! Native-report adapter — the engine-agnostic integration point.
//!
//! Every adapter so far (gate/guard/watch) speaks one specific engine's JSON.
//! The native adapter speaks **zaindari's own contract**: any command, in any
//! language, that emits a schema-versioned envelope wrapping a single
//! [`PillarResult`]. This is the `zaindari.report.json` contract from
//! [[zaindari-unifier-cli-2026-06]] — engines integrate by emitting the
//! envelope, not by zaindari knowing their internals.
//!
//! The envelope (camelCase, matching [`crate::report`]'s serde shape):
//! ```json
//! {
//!   "schemaVersion": 1,
//!   "toolVersion": "berme-eval 0.1.0",   // optional
//!   "pillar": {
//!     "status": "pass",                   // pass|fail|warn|skipped|engine_missing
//!     "headline": "Eval gate held: key-F1 0.97 …",
//!     "metrics": [ { "name": "key_f1", "value": 0.97, "baseline": 0.95,
//!                    "delta": 0.02, "direction": "higher_better" } ],
//!     "findings": [ { "severity": "major", "title": "…", "detail": "…" } ]
//!   }
//! }
//! ```
//! A native emitter is the authority on its own pass/fail — the command's exit
//! code is informational; `pillar.status` decides.

use crate::report::{PillarResult, REPORT_SCHEMA_VERSION};
use serde::Deserialize;

/// The on-disk envelope a native engine writes. A schema-versioned wrapper
/// around one [`PillarResult`] plus an optional engine version string.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeReport {
    schema_version: u32,
    #[serde(default)]
    tool_version: Option<String>,
    pillar: PillarResult,
}

/// Why a native report couldn't be turned into a pillar result.
#[derive(Debug, thiserror::Error)]
pub enum NativeError {
    #[error("native report JSON did not parse: {0}")]
    Parse(#[from] serde_json::Error),
    #[error(
        "native report schemaVersion {got} unsupported (this zaindari understands {expected}) — \
         upgrade zaindari or the emitter"
    )]
    Schema { got: u32, expected: u32 },
}

/// Parse + validate a native envelope into a pillar result and the optional
/// engine version. `raw_ref` is filled in only when the emitter didn't set its
/// own (zaindari knows where it told the command to write).
pub fn parse(
    json: &str,
    raw_ref: Option<String>,
) -> Result<(PillarResult, Option<String>), NativeError> {
    let nr: NativeReport = serde_json::from_str(json)?;
    if nr.schema_version != REPORT_SCHEMA_VERSION {
        return Err(NativeError::Schema {
            got: nr.schema_version,
            expected: REPORT_SCHEMA_VERSION,
        });
    }
    let mut pillar = nr.pillar;
    if pillar.raw_ref.is_none() {
        pillar.raw_ref = raw_ref;
    }
    Ok((pillar, nr.tool_version))
}

/// Install hint shown when a native command's binary is absent.
pub fn install_hint() -> &'static str {
    "the configured `report_cmd` binary was not found — check the command path"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{PillarStatus, Severity};

    fn envelope(status: &str) -> String {
        format!(
            r#"{{
              "schemaVersion": {ver},
              "toolVersion": "berme-eval 0.1.0",
              "pillar": {{
                "status": "{status}",
                "headline": "Eval gate held: key-F1 0.97",
                "metrics": [
                  {{ "name": "key_f1", "value": 0.97, "baseline": 0.95,
                     "delta": 0.02, "direction": "higher_better" }}
                ],
                "findings": [
                  {{ "severity": "major", "title": "value-accuracy dipped",
                     "detail": "0.91 < baseline 0.94" }}
                ]
              }}
            }}"#,
            ver = REPORT_SCHEMA_VERSION,
        )
    }

    #[test]
    fn parses_a_well_formed_envelope() {
        let (pillar, ver) = parse(&envelope("pass"), None).unwrap();
        assert_eq!(pillar.status, PillarStatus::Pass);
        assert_eq!(ver.as_deref(), Some("berme-eval 0.1.0"));
        assert_eq!(pillar.metrics.len(), 1);
        assert!(!pillar.metrics[0].regressed());
        assert_eq!(pillar.findings[0].severity, Severity::Major);
    }

    #[test]
    fn fail_status_round_trips() {
        let (pillar, _) = parse(&envelope("fail"), None).unwrap();
        assert_eq!(pillar.status, PillarStatus::Fail);
    }

    #[test]
    fn zaindari_fills_raw_ref_when_emitter_omits_it() {
        let (pillar, _) = parse(&envelope("pass"), Some("raw/gate-native.json".into())).unwrap();
        assert_eq!(pillar.raw_ref.as_deref(), Some("raw/gate-native.json"));
    }

    #[test]
    fn emitter_raw_ref_wins_over_zaindari_default() {
        let json = format!(
            r#"{{ "schemaVersion": {ver}, "pillar": {{ "status": "pass",
                "headline": "ok", "rawRef": "emitter/path.json" }} }}"#,
            ver = REPORT_SCHEMA_VERSION,
        );
        let (pillar, _) = parse(&json, Some("zaindari/path.json".into())).unwrap();
        assert_eq!(pillar.raw_ref.as_deref(), Some("emitter/path.json"));
    }

    #[test]
    fn wrong_schema_version_is_rejected() {
        let json = r#"{ "schemaVersion": 999, "pillar": { "status": "pass", "headline": "ok" } }"#;
        let err = parse(json, None).unwrap_err();
        assert!(matches!(err, NativeError::Schema { got: 999, .. }));
    }

    #[test]
    fn malformed_json_errors() {
        assert!(parse("{not json", None).is_err());
    }
}
