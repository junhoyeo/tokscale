# 9Router ↔ tokscale Bridge

Bridges 9Router usage data into tokscale via gjc-format JSONL files, enabling
tokscale's analytics, graph, and cost estimation for 9Router API calls.

## Files

- `scripts/9router_tokscale_bridge_gjc.py` — Python bridge script
- `scripts/9router_custom_pricing.json` — Custom pricing for non-free models
- `docs/9router-bridge.md` — This file

## Setup

### 1. Merge custom pricing

> **Warning:** Do not `cp` this file over `~/.config/tokscale/custom-pricing.json`
> — if you already have overrides there, a plain copy silently destroys them.
> Merge the 9Router entries in instead. If the file doesn't exist yet, this
> also handles creating it.
>
> Custom pricing is global and exact-match: `scripts/9router_custom_pricing.json`
> ships the generic model ids `kimi-k2.5` and `gpt-oss-120b`, which could
> collide with entries you already have. The merge below keeps YOUR existing
> entries on any key collision.

```bash
mkdir -p ~/.config/tokscale
if [ -f ~/.config/tokscale/custom-pricing.json ]; then
  # Object addition in jq is right-biased (right operand wins on key
  # collisions), so the user's existing models must be the RIGHT operand.
  jq -s '.[0].models = (.[1].models + .[0].models) | .[0]' \
    ~/.config/tokscale/custom-pricing.json scripts/9router_custom_pricing.json \
    > ~/.config/tokscale/custom-pricing.json.tmp \
  && mv ~/.config/tokscale/custom-pricing.json.tmp ~/.config/tokscale/custom-pricing.json
else
  cp scripts/9router_custom_pricing.json ~/.config/tokscale/custom-pricing.json
fi
```

Alternatively, open `~/.config/tokscale/custom-pricing.json` and manually
append the entries from `scripts/9router_custom_pricing.json`'s `models` key.

### 2. Configure tokscale scanner

Add the bridge output directory to your tokscale settings file (default
`~/.config/tokscale/settings.json`; override with `TOKSCALE_CONFIG_DIR` or
`XDG_CONFIG_HOME`):

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
tokscale graph --client 9router
tokscale models --client 9router
tokscale pricing "deepseek-ai/deepseek-v4-flash"
```

Bridge messages are stamped with client `9router`. `--client 9router`
shows only bridge data; `--client gjc` is a superset that shows native
gjc sessions *plus* bridge data (9Router data is gjc-format).

## How It Works

1. Bridge reads 9Router's SQLite database (`~/.9router/db/data.sqlite`)
2. Extracts token usage from `requestDetails` table
3. Writes gjc-format JSONL files grouped by date to `~/.local/share/9router-tokscale/sessions/`
4. Tokscale's gjc parser reads these files and applies pricing from its database + custom pricing

## Critical: Cost Field Policy

For **paid models**, the bridge intentionally omits `usage.cost` from JSONL
output. The gjc parser (`sessions/gjc.rs:embedded_cost`) treats any present
`cost.total` — even `0.0` — as `CostSource::ProviderReported` (authoritative).
This prevents `apply_pricing_if_available` from repricing via the pricing
database. Omitting the cost field causes `embedded_cost` to return
`(0.0, CostSource::Unknown)`, allowing the dispatch guard to reprice from
tokens + pricing data.

For **free-tier models** (ids ending in `-free` or `:free`, case-insensitive),
the bridge does the opposite: it embeds `"cost": {"total": 0.0}`. Tokscale's
pricing lookup strips the `-free` suffix before matching, so an omitted cost
would reprice e.g. `kimi-k2.5-free` at the *paid* `kimi-k2.5` rate. The
embedded authoritative `$0.00` pins free usage at zero cost while tokens are
still counted.

## Provider Inference

For tokscale's pricing lookup to work, the bridge reads the `provider` column
from 9Router's database. When the provider field is empty and the model ID
contains a `/` (e.g. `deepseek-ai/deepseek-v4-flash`), the bridge derives the
provider hint from the first path segment:

| Model ID | Provider |
|---|---|
| `deepseek-ai/deepseek-v4-flash` | `deepseek-ai` |
| `stepfun-ai/step-3.7-flash` | `stepfun-ai` |
| `@cf/moonshotai/kimi-k2.5` | `moonshotai` (from DB) |
| `mimo-v2.5` | `mimo-v2.5` (no `/`, passes through DB value) |

When 9Router provides the provider directly, it is used as-is.

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
file only covers paid models. For free models the bridge embeds an
authoritative `"cost": {"total": 0.0}` in each row (see "Critical: Cost Field
Policy" above), so they show `$0.00` cost instead of being repriced at the
paid base-model rate after tokscale strips the `-free` suffix.

## Known Limitations

- Ollama models are omitted when the upstream API returns no usage metadata
- Bridge only rewrites the per-date files that have rows in the current DB
  scan; date files with no matching rows on a run are left untouched, so
  stale date files can linger if the underlying data disappears (e.g. after
  a DB is pruned or replaced)

## Automation

A systemd user timer runs the bridge every 10 minutes automatically so
`tokscale --today` stays current without manual intervention.

### One-time setup

```bash
# Install the bridge script where the systemd unit expects it
mkdir -p ~/.local/share/9router-tokscale/
cp ~/Documents/Rust/tokscale/scripts/9router_tokscale_bridge_gjc.py ~/.local/share/9router-tokscale/9router_tokscale_bridge_gjc.py
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
