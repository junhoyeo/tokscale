# 9Router ↔ tokscale Bridge

Bridges 9Router usage data into tokscale via gjc-format JSONL files, enabling
tokscale's analytics, graph, and cost estimation for 9Router API calls.

## Files

- `scripts/9router_tokscale_bridge_gjc.py` — Python bridge script
- `scripts/9router_custom_pricing.json` — Custom pricing for non-free models
- `docs/9router-bridge.md` — This file

## Setup

### 1. Copy custom pricing

```bash
cp scripts/9router_custom_pricing.json ~/.config/tokscale/custom-pricing.json
```

### 2. Configure tokscale scanner

Add the bridge output directory to `~/.config/tokscale/settings.json`:

```json
{
  "scanner": {
    "extraScanPaths": {
      "gjc": ["/home/USER/.local/share/9router-tokscale/sessions"]
    }
  }
}
```

### 3. Run the bridge

```bash
python3 scripts/9router_tokscale_bridge_gjc.py
```

### 4. Verify

```bash
tokscale graph --client gjc
tokscale models --client gjc
tokscale pricing "deepseek-ai/deepseek-v4-flash"
```

## How It Works

1. Bridge reads 9Router's SQLite database (`~/.9router/db/data.sqlite`)
2. Extracts token usage from `requestDetails` table
3. Writes gjc-format JSONL files grouped by date to `~/.local/share/9router-tokscale/sessions/`
4. Tokscale's gjc parser reads these files and applies pricing from its database + custom pricing

## Critical: Cost Field Omission

The bridge intentionally omits `usage.cost` from JSONL output. The gjc parser
(`sessions/gjc.rs:embedded_cost`) treats any present `cost.total` — even `0.0` —
as `CostSource::ProviderReported` (authoritative). This prevents
`apply_pricing_if_available` from repricing via the pricing database.

Omitting the cost field causes `embedded_cost` to return `(0.0, CostSource::Unknown)`,
allowing the dispatch guard to reprice from tokens + pricing data.

## Provider Derivation

For tokscale's pricing lookup to work, the bridge derives `provider` from the
model ID's first segment:

| Model ID | Provider |
|---|---|
| `deepseek-ai/deepseek-v4-flash` | `deepseek-ai` |
| `stepfun-ai/step-3.7-flash` | `stepfun-ai` |
| `@cf/moonshotai/kimi-k2.5` | `moonshotai` |
| `mimo-v2.5` | (empty) |

This makes `provider_hint_matches_scoped_provider` succeed in the lookup chain.

## 9Router Model Prefixes

9Router uses these prefixes for different providers:

| Prefix | Provider |
|---|---|
| `nvidia/` | NVIDIA NIM |
| `kc/` | Kilocode |
| `cf/` | Cloudflare Workers AI |
| `gh/` | GitHub Models |
| `cx/` | Codex |
| `ollama/` | Ollama (local) |
| `openrouter/` | OpenRouter |
| `gemini/` | Google Gemini |
| `gc/` | Gemini CLI |
| `bpm/` | BytePlus ModelArk |
| `cerebras/` | Cerebras |

## Free vs Paid Models

Models with `:free` suffix or `*-free` suffix are free tier. The custom pricing
file only covers paid models. Free models correctly show `$0.00` cost.

## Known Limitations

- Ollama models report 0 tokens (upstream API doesn't return usage metadata)
- Bridge regenerates all files on each run (full refresh)

## Automation

A systemd user timer runs the bridge every 10 minutes automatically so
`tokscale --today` stays current without manual intervention.

### One-time setup

```bash
mkdir -p ~/.config/systemd/user/
cp ~/Documents/Rust/tokscale/scripts/systemd/9router-tokscale-bridge.{service,timer} ~/.config/systemd/user/
loginctl enable-linger $USER   # so timers run without an active login session
systemctl --user daemon-reload
systemctl --user enable --now 9router-tokscale-bridge.timer
# Verify:
systemctl --user list-timers | grep 9router
systemctl --user start 9router-tokscale-bridge.service   # first run, no waiting
journalctl --user -u 9router-tokscale-bridge.service -n 20
```

### How it works

The timer fires `9router-tokscale-bridge.service` every 10 minutes. Each run
reads the 9Router SQLite DB and writes gjc-format JSONL files grouped by local
date. The `Persistent=true` setting ensures missed runs (hibernate, reboot) are
caught up immediately on next boot.

### Troubleshooting

- Check timer status: `systemctl --user status 9router-tokscale-bridge.timer`
- Check last run: `journalctl --user -u 9router-tokscale-bridge.service -n 20`
- Reinstall units: repeat the one-time setup commands above (units are overwritten in place)
