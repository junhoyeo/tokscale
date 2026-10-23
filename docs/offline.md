# Offline reports and pricing caches

Use `tokscale --offline` to open the TUI without network requests. The flag is global and also works after a subcommand:

```sh
tokscale models --offline --json --no-spinner
tokscale monthly --offline --light --no-spinner
tokscale graph --offline --no-spinner
tokscale pricing claude-sonnet-4-20250514 --offline --json --no-spinner
tokscale --no-spinner report --offline --no-summarize
```

Offline mode reads existing session files, source caches, custom pricing and any usable cached pricing catalogs regardless of their age. It does not refresh Cursor or Amp history, quotas, server statistics or Wrapped assets. TUI quota cards can display an existing cache; refresh, Codex login and limit-reset actions cannot start network work. Wrapped requires cached fonts from an earlier online run; optional uncached images are omitted. `report` requires `--no-summarize` because summarizers may invoke external network-capable programs.

Unknown prices remain unknown and may produce an incomplete-cost notice. A zero total with that notice is not a claim that the usage was free. Offline mode does not backdate, renew or silently overwrite catalog timestamps. Missing remote-only history is not reconstructed. Local report/source caches may still be written normally.

Commands requiring online access, including submission (also dry-run), login, usage fetching, explicit history sync and pricing refresh, reject `--offline` before their work starts. The option is a Tokscale execution policy, not an operating-system network sandbox.

## Preparing or transferring prices

While connected, run:

```sh
tokscale pricing refresh --json --no-spinner
```

This bypasses the normal freshness TTL for LiteLLM, OpenRouter and models.dev. Each validated catalog is saved with an atomic replacement. A failed fetch or save is reported per source, the command exits unsuccessfully if any source failed, and previous copies of failed catalogs remain intact. Successful catalogs can still be used. Refresh does not download account credentials or remote session history.

Catalog files are JSON envelopes with a Unix `timestamp` and a `data` object. They live in Tokscale's canonical configuration directory under `cache/`: `pricing-litellm.json`, `pricing-openrouter.json` and `pricing-models-dev.json`. The default configuration directory is `~/.config/tokscale` on macOS, the XDG configuration directory plus `tokscale` on Linux, and the platform configuration directory plus `tokscale` on Windows. `TOKSCALE_CONFIG_DIR` overrides it on all platforms. Copy these files to the same relative locations on an offline machine. Keep their timestamps unchanged. Copy `custom-pricing.json` separately if needed, after checking that it contains the intended billing-provider rules. No credential files are needed for offline price lookup.

The existing `TOKSCALE_PRICING_CACHE_ONLY` environment variable controls only certain local pricing paths. It is not a substitute for `--offline`, which also disables implicit synchronization and interactive network actions.
