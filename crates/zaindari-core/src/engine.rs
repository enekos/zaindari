//! Engine invocation: run a binary, capture stdout + exit code, detect a
//! missing binary. Kept tiny and separate so the *parsing* in each adapter
//! stays pure and unit-testable against fixtures without spawning anything.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// What an engine invocation produced. Adapters parse `stdout` (and sometimes
/// the exit code) into the shared report model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineRun {
    pub stdout: String,
    pub stderr: String,
    /// `None` if the process was terminated by a signal.
    pub exit_code: Option<i32>,
}

/// Why an engine couldn't be run.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The binary was not found / not executable. Adapters turn this into an
    /// `engine_missing` pillar, not a hard failure.
    #[error("engine binary `{0}` not found or not executable")]
    Missing(String),
    /// The binary was found but spawning/waiting failed for another reason.
    #[error("running `{bin}`: {source}")]
    Spawn {
        bin: String,
        #[source]
        source: std::io::Error,
    },
}

/// Run `bin` with `args` in `cwd`. Captures stdout/stderr.
///
/// A `NotFound` io error maps to [`EngineError::Missing`] so the caller can
/// distinguish "you haven't installed this engine" from "this engine failed".
pub fn run<I, S>(bin: &str, args: I, cwd: &Path) -> Result<EngineRun, EngineError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(bin)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                EngineError::Missing(bin.to_string())
            } else {
                EngineError::Spawn {
                    bin: bin.to_string(),
                    source: e,
                }
            }
        })?;
    Ok(EngineRun {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code(),
    })
}
