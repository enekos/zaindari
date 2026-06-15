//! `zaindari.toml` — the one config over the three engines.
//!
//! Discovered by walking up from the working directory. Every pillar section
//! is optional: a missing `[gate]` means the gate pillar is reported
//! [`crate::report::PillarStatus::Skipped`], never failed.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Errors raised while locating or parsing the config.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("no zaindari.toml found from {0} upward to the filesystem root")]
    NotFound(PathBuf),
    #[error("reading {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

/// The whole config. Each pillar section is independently optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<GateConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<GuardConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<WatchConfig>,
}

/// Gate pillar — `aatxe evals`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GateConfig {
    /// Path to the `aatxe` binary; defaults to the bare name on PATH.
    #[serde(default = "default_aatxe_bin")]
    pub bin: String,
    /// Council corpus directory passed as `--corpus`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corpus: Option<PathBuf>,
    /// Baseline eval JSON passed as `--baseline`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<PathBuf>,
    /// Extra flags appended verbatim to the `aatxe evals` invocation
    /// (e.g. `--council`, `--stats`, `--confidence-floor 0.3`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

/// Guard pillar — `iratxo test`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GuardConfig {
    /// Path to the `iratxo` binary; defaults to the bare name on PATH.
    #[serde(default = "default_iratxo_bin")]
    pub bin: String,
    /// Pack / suite / directory paths passed to `iratxo test`.
    pub packs: Vec<PathBuf>,
}

/// Watch pillar — `cardinal-map check`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WatchConfig {
    /// Path to the `cardinal-map` binary; defaults to the bare name on PATH.
    #[serde(default = "default_cardinal_bin")]
    pub bin: String,
    /// Trained-profile directory passed as `--profiles`.
    pub profiles: PathBuf,
    /// Entity schema JSON passed as `--schema`.
    pub schema: PathBuf,
    /// JSON array of names to score, passed as `--input`.
    pub input: PathBuf,
    /// Cardinality threshold above which an item is flagged anomalous.
    #[serde(default = "default_watch_threshold")]
    pub anomaly_threshold: f64,
}

fn default_aatxe_bin() -> String {
    "aatxe".to_string()
}
fn default_iratxo_bin() -> String {
    "iratxo".to_string()
}
fn default_cardinal_bin() -> String {
    "cardinal-map".to_string()
}
fn default_watch_threshold() -> f64 {
    0.6
}

/// The filename zaindari looks for when walking up.
pub const CONFIG_FILENAME: &str = "zaindari.toml";

impl Config {
    /// Parse a config from a TOML string.
    pub fn from_toml_str(src: &str, path: &Path) -> Result<Self, ConfigError> {
        toml::from_str(src).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Load and parse the config at an explicit path.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let src = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml_str(&src, path)
    }

    /// Walk up from `start` until a [`CONFIG_FILENAME`] is found, then load it.
    /// Returns the parsed config and the path it was read from.
    pub fn discover(start: &Path) -> Result<(Self, PathBuf), ConfigError> {
        let path =
            find_config_path(start).ok_or_else(|| ConfigError::NotFound(start.to_path_buf()))?;
        let cfg = Self::load(&path)?;
        Ok((cfg, path))
    }
}

/// Walk up from `start` (a directory) looking for [`CONFIG_FILENAME`].
/// Pure path logic against the real filesystem; returns the first hit.
pub fn find_config_path(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let candidate = d.join(CONFIG_FILENAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}

/// The commented sample config `zaindari init` writes.
pub fn sample_config() -> &'static str {
    SAMPLE_CONFIG
}

const SAMPLE_CONFIG: &str = r#"# zaindari.toml — one config over three LLM-trust engines.
# Every section is optional. A missing section means that pillar is reported
# "skipped", never failed. Delete the sections you don't use.

# ── Gate: pre-ship eval regression gate (engine: aatxe) ──────────────────────
[gate]
# bin = "aatxe"                 # binary path; defaults to `aatxe` on PATH
corpus = "evals/council/cases"  # council corpus dir (--corpus)
baseline = "evals/baseline.json" # baseline eval JSON (--baseline); regression -> exit 2
flags = ["--council", "--stats"] # appended verbatim to `aatxe evals`

# ── Guard: runtime rule packs (engine: iratxo) ───────────────────────────────
[guard]
# bin = "iratxo"
packs = ["rules/promo.cases.yaml"] # suite / pack / dir paths for `iratxo test`

# ── Watch: post-ship drift detection (engine: cardinal-map) ──────────────────
[watch]
# bin = "cardinal-map"
profiles = "profiles/product"   # trained-profile dir (--profiles)
schema = "schemas/product.json" # entity schema (--schema)
input = "watch/today.json"      # JSON array of names to score (--input)
anomaly_threshold = 0.6         # cardinality >= this is flagged anomalous
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_parses_with_all_pillars_absent() {
        let cfg = Config::from_toml_str("", Path::new("zaindari.toml")).unwrap();
        assert!(cfg.gate.is_none());
        assert!(cfg.guard.is_none());
        assert!(cfg.watch.is_none());
    }

    #[test]
    fn pillar_bins_default_to_bare_names() {
        let src = r#"
[gate]
[guard]
packs = ["a.yaml"]
[watch]
profiles = "p"
schema = "s.json"
input = "i.json"
"#;
        let cfg = Config::from_toml_str(src, Path::new("zaindari.toml")).unwrap();
        assert_eq!(cfg.gate.unwrap().bin, "aatxe");
        assert_eq!(cfg.guard.unwrap().bin, "iratxo");
        let w = cfg.watch.unwrap();
        assert_eq!(w.bin, "cardinal-map");
        assert_eq!(w.anomaly_threshold, 0.6);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = Config::from_toml_str("[gate]\nbogus = 1\n", Path::new("zaindari.toml"));
        assert!(err.is_err());
    }

    #[test]
    fn sample_config_is_valid_toml() {
        let cfg = Config::from_toml_str(sample_config(), Path::new("zaindari.toml")).unwrap();
        // All three sample sections present and parse.
        assert!(cfg.gate.is_some());
        assert!(cfg.guard.is_some());
        assert!(cfg.watch.is_some());
    }
}
