# Copilot CLI Guide

This guide explains how to use the new GitHub Copilot CLI support in `tokscale`.

## 1. Enable Copilot OTEL export

Copilot must write OpenTelemetry JSONL locally before `tokscale` can read it.

```bash
export COPILOT_OTEL_ENABLED=true
export COPILOT_OTEL_EXPORTER_TYPE=file
export COPILOT_OTEL_FILE_EXPORTER_PATH="$HOME/.copilot/otel/copilot-otel.jsonl"
mkdir -p "$HOME/.copilot/otel"
```

Then use Copilot CLI normally. Its `chat` spans will be appended to the OTEL file.

## 2. Run `tokscale`

From the repository root:

```bash
cargo +1.88.0 run -p tokscale-cli -- clients
cargo +1.88.0 run -p tokscale-cli -- --copilot
```

If you want a built binary instead:

```bash
cargo +1.88.0 build -p tokscale-cli
./target/debug/tokscale --copilot
```

## 3. What the commands do

| Command | What it does |
| --- | --- |
| `cargo +1.88.0 run -p tokscale-cli -- clients` | Shows where `tokscale` looks for local data, including the Copilot OTEL path it discovered. |
| `cargo +1.88.0 run -p tokscale-cli -- --copilot` | Runs the normal report, filtered to Copilot only. |
| `cargo +1.88.0 run -p tokscale-cli -- --copilot --json --no-spinner` | Prints Copilot-only usage as JSON for scripting or inspection. |
| `cargo +1.88.0 run -p tokscale-cli -- models --copilot` | Shows model-level Copilot usage totals. |
| `cargo +1.88.0 run -p tokscale-cli -- graph --copilot --output copilot-graph.json` | Exports Copilot-only graph data as JSON. |
| `cargo +1.88.0 run -p tokscale-cli -- wrapped --copilot` | Generates a Copilot-only wrapped/year-in-review view. |

## 4. What `tokscale` reads from Copilot

Current support is intentionally focused on token and session analytics.

- `tokscale` reads Copilot OTEL **`chat` spans**
- it extracts:
  - input tokens
  - output tokens
  - cache-read tokens
  - reasoning tokens
- it ignores tool spans and cumulative OTEL metrics for now

## 5. Pricing behavior

`tokscale` does **not** trust `github.copilot.cost` directly.

Instead:

- the client is recorded as `copilot`
- the provider is inferred from the model when possible
- normal model pricing is applied when that pricing is available

## 6. Troubleshooting

| Symptom | What to check |
| --- | --- |
| No Copilot data appears | Make sure the OTEL env vars are set before running Copilot CLI. |
| `clients` does not show Copilot | Check that `~/.copilot/otel/` exists or that `COPILOT_OTEL_FILE_EXPORTER_PATH` points to a real file. |
| Report is empty | Generate a short Copilot session first so the exporter writes at least one `chat` span. |
| `cargo +1.88.0 ...` fails | Install the toolchain with `rustup toolchain install 1.88.0`. |

## 7. Quick copy-paste flow

```bash
export COPILOT_OTEL_ENABLED=true
export COPILOT_OTEL_EXPORTER_TYPE=file
export COPILOT_OTEL_FILE_EXPORTER_PATH="$HOME/.copilot/otel/copilot-otel.jsonl"
mkdir -p "$HOME/.copilot/otel"

# use Copilot CLI here

cd /Users/patrickmcgannon/prog/tokscale
cargo +1.88.0 run -p tokscale-cli -- clients
cargo +1.88.0 run -p tokscale-cli -- --copilot
```
