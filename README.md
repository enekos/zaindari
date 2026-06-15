# zaindari

**One CLI over three LLM-trust engines.** Gate your AI feature in CI, enforce
hard rules at runtime, and watch for output drift in production — from a single
config and a single PM-readable report.

zaindari is an **orchestrator**, not a reimplementation. It drives three
existing engines, captures their output, and maps everything into one shared
report model with one exit-code policy:

| Pillar | When | Question it answers | Engine |
|---|---|---|---|
| **Gate** | pre-ship (CI) | "Did this prompt/model change make eval quality worse?" | [aatxe](#engines) |
| **Guard** | runtime | "Does the output violate a hard, portable rule?" | [iratxo](#engines) |
| **Watch** | post-ship | "Is a live output anomalous vs the trained baseline — without labels?" | [cardinal-map](#engines) |

The story: **Gate** stops a regression before it ships; **Guard** is the
deterministic safety net the LLM can't talk its way past; **Watch** catches the
drift that only shows up on real traffic. zaindari runs whichever of the three
you've configured and rolls the result into one HTML scorecard a non-engineer
can read.

## Quickstart

```sh
# 1. scaffold a config in your repo
zaindari init            # writes a commented zaindari.toml

# 2. edit zaindari.toml — keep only the pillars you use, point each at its inputs

# 3. run all configured pillars; save the report; render it for humans
zaindari run --out report.json
zaindari report report.json --html report.html
```

Each pillar can also run on its own: `zaindari gate`, `zaindari guard`,
`zaindari watch`. The config is discovered by walking up from the working
directory, so subcommands work from anywhere inside the project.

## Configuration

`zaindari.toml` — every section is optional. A **missing** section means that
pillar is reported `skipped`, never `failed`.

```toml
[gate]                              # engine: aatxe
# bin = "aatxe"                     # defaults to `aatxe` on PATH
corpus = "evals/council/cases"      # --corpus
baseline = "evals/baseline.json"    # --baseline; a regression past tolerance -> exit 2
flags = ["--council", "--stats"]    # appended verbatim to `aatxe evals`

[guard]                             # engine: iratxo
packs = ["rules/promo.cases.yaml"]  # suite / pack / dir paths for `iratxo test`

[watch]                             # engine: cardinal-map
profiles = "profiles/product"       # --profiles (trained store)
schema = "schemas/product.json"     # --schema
input = "watch/today.json"          # --input (JSON array of names to score)
anomaly_threshold = 0.6             # cardinality >= this is flagged anomalous
```

## Exit codes

CI-gate semantics: a non-zero code fails the pipeline.

| Code | Meaning |
|---|---|
| `0` | All configured pillars passed (or were skipped). Watch anomalies map here by default. |
| `2` | **Gate or Guard failed** — eval regression, or a rule case failed. Block the build. |
| `1` | An engine errored or its binary was missing. The run is inconclusive. |

`watch` anomalies are a **warning (exit 0)** by default — drift is signal to
review, not an automatic build break. Pass `--strict-watch` to promote watch
anomalies to a gating failure (exit 2).

Global flags: `--json` (emit the machine-readable report to stdout),
`--out <path>` (write the report JSON), `--strict-watch`.

## Report model

Every engine's output is mapped into one shape (`zaindari-core::report`):

```
ZaindariReport { schema_version, tool_versions, pillars: { gate?, guard?, watch? } }
PillarResult   { status: pass|fail|warn|skipped|engine_missing,
                 headline, metrics[], findings[], raw_ref }
Metric         { name, value, baseline?, delta?, direction }
Finding        { severity, title, detail, location? }
```

The HTML report (`zaindari report --html`) is a single self-contained file —
inline CSS, no JavaScript, no CDN — with a traffic light and a plain-English
headline per pillar, metric tables showing baseline deltas, and findings
grouped by severity.

## Engines

zaindari does not vendor or reimplement these — install them separately and
point the config at each binary (or rely on the bare name being on `PATH`).

- **Gate — [aatxe](https://github.com/enekos/aatxe)**: eval harness with
  regression baselines and an LLM review council. zaindari runs
  `aatxe evals --out <json> [--baseline <json>]` and reads the eval JSON.
- **Guard — iratxo**: YAML→IR→WASM rule engine; portable signed rule packs.
  zaindari runs `iratxo test <packs…>` and parses its result.
- **Watch — cardinal-map**: label-free anomaly detection on LLM extractions.
  zaindari runs `cardinal-map check --json` and reads the per-item scores.

If an engine binary is absent, that pillar is reported `engine_missing` with an
install hint — zaindari never panics on a missing engine.

## v0 = orchestrator

This release is deliberately a thin orchestration layer: it shells out to the
three engine binaries and unifies their output. It does **not** embed the
engines as libraries or run any model itself.

**Roadmap**
- **Library bindings** — link the engines in-process (no subprocess, no JSON
  round-trip) once their public APIs stabilise.
- **Machine-readable Guard output** — `iratxo test` emits human text today;
  a `--json` mode would make the Guard adapter robust (tracked as a follow-up).
- **Hosted** — a dashboard, a signed domain-pack registry, and a cross-repo
  learning corpus, after design partners ask for it.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace        # passes WITHOUT any engine binary installed
```

The workspace is two crates: `zaindari-core` (pure logic — config, report
model, adapters, render, policy) and `zaindari-cli` (the thin `zaindari`
binary). Adapter parsing is pure and unit-tested against hand-derived fixtures
that mirror the real engine output shapes; no test invokes a real engine or an
LLM.

## License

MIT — see [LICENSE](LICENSE).
