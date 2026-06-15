//! `zaindari` — thin CLI wrapper around [`zaindari_core`].
//!
//! Arg parsing + wiring only; all logic lives in the library so it stays
//! testable and reusable. The binary maps a finished report to a process exit
//! code via [`zaindari_core::policy`].

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;
use zaindari_core::{
    config::{self, Config},
    policy::{self, PolicyOptions},
    render,
    report::ZaindariReport,
    run::{self, RunContext, Selection},
};

#[derive(Parser)]
#[command(
    name = "zaindari",
    version,
    about = "One CLI over three LLM-trust engines: Gate (evals), Guard (rules), Watch (drift)."
)]
struct Cli {
    /// Emit the machine-readable JSON report to stdout instead of the text summary.
    #[arg(long, global = true)]
    json: bool,

    /// Write the report (JSON) to this path.
    #[arg(long, global = true, value_name = "PATH")]
    out: Option<PathBuf>,

    /// Promote watch anomalies (WARN) to a gating failure (exit 2).
    #[arg(long, global = true)]
    strict_watch: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the Gate pillar (pre-ship eval regression gate).
    Gate,
    /// Run the Guard pillar (runtime rule packs).
    Guard,
    /// Run the Watch pillar (post-ship drift detection).
    Watch,
    /// Run every configured pillar.
    Run,
    /// Render a previously-saved run JSON; optionally to a self-contained HTML file.
    Report {
        /// Path to a saved zaindari run JSON.
        run_json: PathBuf,
        /// Write a self-contained HTML report here.
        #[arg(long, value_name = "FILE")]
        html: Option<PathBuf>,
    },
    /// Write a commented sample zaindari.toml into the current directory.
    Init {
        /// Overwrite an existing zaindari.toml.
        #[arg(long)]
        force: bool,
    },
}

fn main() -> ExitCode {
    match real_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("zaindari: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn real_main() -> Result<ExitCode> {
    let cli = Cli::parse();

    // `init` and `report` don't run engines and never gate — handle first.
    match &cli.command {
        Command::Init { force } => return cmd_init(*force).map(|()| ExitCode::SUCCESS),
        Command::Report { run_json, html } => {
            return cmd_report(run_json, html.as_ref(), cli.json).map(|()| ExitCode::SUCCESS)
        }
        _ => {}
    }

    let selection = match &cli.command {
        Command::Gate => Selection::only_gate(),
        Command::Guard => Selection::only_guard(),
        Command::Watch => Selection::only_watch(),
        Command::Run => Selection::all(),
        Command::Init { .. } | Command::Report { .. } => unreachable!("handled above"),
    };

    let cwd = std::env::current_dir().context("getting current directory")?;
    let (cfg, cfg_path) = Config::discover(&cwd)
        .with_context(|| "no zaindari.toml found — run `zaindari init` to create one")?;
    let base_dir = cfg_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| cwd.clone());

    let ctx = RunContext {
        cwd: &base_dir,
        raw_dir: None,
    };
    let report = run::run(&cfg, selection, &ctx);

    emit_report(&report, cli.json, cli.out.as_ref())?;

    let opts = PolicyOptions {
        strict_watch: cli.strict_watch,
        error_on_missing: true,
    };
    Ok(ExitCode::from(policy::exit_code(&report, opts) as u8))
}

fn cmd_init(force: bool) -> Result<()> {
    let path = PathBuf::from(config::CONFIG_FILENAME);
    if path.exists() && !force {
        anyhow::bail!(
            "{} already exists — pass --force to overwrite",
            path.display()
        );
    }
    std::fs::write(&path, config::sample_config())
        .with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

fn cmd_report(run_json: &PathBuf, html: Option<&PathBuf>, json: bool) -> Result<()> {
    let raw = std::fs::read_to_string(run_json)
        .with_context(|| format!("reading {}", run_json.display()))?;
    let report: ZaindariReport = serde_json::from_str(&raw)
        .with_context(|| format!("parsing run JSON {}", run_json.display()))?;

    if let Some(html_path) = html {
        let body = render::to_html(&report);
        std::fs::write(html_path, body)
            .with_context(|| format!("writing HTML to {}", html_path.display()))?;
        eprintln!("wrote {}", html_path.display());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if html.is_none() {
        print!("{}", render::to_text(&report));
    }
    Ok(())
}

fn emit_report(report: &ZaindariReport, json: bool, out: Option<&PathBuf>) -> Result<()> {
    let json_body = serde_json::to_string_pretty(report)?;
    if let Some(path) = out {
        std::fs::write(path, &json_body).with_context(|| format!("writing {}", path.display()))?;
        eprintln!("wrote {}", path.display());
    }
    if json {
        println!("{json_body}");
    } else {
        print!("{}", render::to_text(report));
    }
    Ok(())
}
