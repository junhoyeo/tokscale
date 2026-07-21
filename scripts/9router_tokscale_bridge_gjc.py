#!/usr/bin/env python3
"""Bridge 9Router usage into tokscale via gjc-format JSONL files.

Reads 9Router request details from ~/.9router/db/data.sqlite (and all
backup DBs from previous upgrades) and writes JSONL files that tokscale's
gjc client parser can consume.

Usage:
    python3 scripts/9router_tokscale_bridge_gjc.py

Then add to ~/.config/tokscale/settings.json:
    {"scanner": {"extraScanPaths": {"gjc": ["/home/USER/.local/share/9router-tokscale/sessions"]}}}

CRITICAL cost policy: for PAID models, do NOT emit usage.cost in the JSONL
output. The gjc parser treats any present cost.total (even 0.0) as
CostSource::ProviderReported, which prevents tokscale from repricing via its
pricing database. Omitting the cost field lets tokscale reprice from
tokens + pricing data.

For FREE-tier models (ids ending in "-free" or ":free"), the bridge embeds
"cost": {"total": 0.0} on purpose: tokscale's pricing lookup strips the
"-free" suffix, so an omitted cost would reprice e.g. kimi-k2.5-free at the
PAID kimi-k2.5 rate. The authoritative $0.00 pins free usage at zero cost
while tokens still count.

See docs/9router-bridge.md for full documentation.
"""

import json
import os
import tempfile
import sqlite3
from pathlib import Path
from datetime import datetime, timezone
from urllib.parse import quote

ROUTER_DB = Path.home() / ".9router" / "db" / "data.sqlite"
BRIDGE_DIR = Path.home() / ".local" / "share" / "9router-tokscale" / "sessions"

def discover_router_dbs() -> list[Path]:
    """Discover the current 9Router DB and all backup DBs.

    Backups live in ~/.9router/db/backups/<upgrade-info>/data.sqlite.
    Returns paths sorted newest-first (by mtime), so the current DB
    (most recently written) is queried first and its request IDs win
    dedup against older backups.
    """
    dbs = [ROUTER_DB]
    backup_glob = ROUTER_DB.parent / "backups"
    if backup_glob.exists():
        dbs.extend(sorted(
            backup_glob.glob("*/data.sqlite"),
            key=lambda p: p.stat().st_mtime,
            reverse=True,
        ))
    return [db for db in dbs if db.exists()]

def ensure_bridge_dir():
    """Create output directory. Existing files are NOT deleted — each date's
    file is overwritten individually by the write loop, preserving historical
    data from previous bridge runs."""
    BRIDGE_DIR.mkdir(parents=True, exist_ok=True)

def open_readonly_db(db_path: Path) -> sqlite3.Connection | None:
    """Open a 9Router DB read-only via a `file:` URI.

    A plain `sqlite3.connect(path)` opens read-write and creates the file
    if it doesn't exist, so a renamed/missing DB path would silently
    produce an empty database instead of failing. Returns None (with a
    guard for the missing-file case) rather than raising.
    """
    if not db_path.exists():
        return None
    conn = sqlite3.connect(f"file:{quote(str(db_path))}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn

def parse_iso_timestamp(ts: str) -> int | None:
    """Convert ISO-8601 timestamp to Unix milliseconds.

    Returns None for missing/unparseable timestamps rather than defaulting
    to now(). Session IDs are date-based (e.g. "9router-2026-01-15") and
    old DB/backup files linger indefinitely, so defaulting to now() would
    give the same malformed row a NEW dedup key every day it's re-read,
    causing perpetual re-duplication instead of a one-time skip.
    """
    if not ts:
        return None
    try:
        dt = datetime.fromisoformat(ts.replace("Z", "+00:00"))
        return int(dt.timestamp() * 1000)
    except Exception:
        return None


def safe_int(value) -> int:
    """Coerce a token scalar to a non-negative int.

    Handles non-numeric values (e.g. a stringified `"100"`, `None`, or
    garbage) by returning 0 instead of raising, and clamps negatives to 0
    so a corrupt negative `cached_tokens` can't inflate computed input.
    """
    try:
        n = int(value)
    except (TypeError, ValueError):
        return 0
    return max(n, 0)

def compute_token_buckets(tokens: dict) -> tuple[int, int, int, int]:
    """Split OpenAI-style token counts into non-overlapping buckets.

    OpenAI's `prompt_tokens` already includes `cached_tokens`, so the
    non-cached input bucket is `max(prompt - cached, 0)` (clamped so a
    `cached_tokens` value larger than `prompt_tokens` never goes negative).

    Token scalars are coerced via `safe_int` so a non-numeric value (e.g.
    a stringified `"100"`) becomes 0 instead of raising, and negatives are
    clamped to 0 at ingestion.

    Returns (input_tokens, cached, completion, total).
    """
    prompt = safe_int(tokens.get("prompt_tokens", 0))
    completion = safe_int(tokens.get("completion_tokens", 0))
    cached = safe_int(tokens.get("cached_tokens", 0))

    input_tokens = max(prompt - cached, 0)
    total = input_tokens + cached + completion
    return input_tokens, cached, completion, total

def is_free_model(model: str) -> bool:
    """Return True for free-tier model ids (ending in "-free" or ":free").

    Tokscale's pricing lookup strips the "-free" suffix before matching, so
    a free variant left without an embedded cost would be repriced at the
    PAID base-model rate. Free rows therefore embed an authoritative $0.00.
    """
    lowered = model.lower()
    return lowered.endswith("-free") or lowered.endswith(":free")

def convert_row_to_entry(row, stats: dict | None = None) -> dict | None:
    """Convert a single `requestDetails` DB row into a gjc-format entry.

    Returns None when the row should be skipped: malformed/NULL `data`
    JSON, a non-dict `tokens` field, zero prompt and completion tokens, or
    a missing/unparseable timestamp. When `stats` is given, a skipped-for-
    timestamp row increments `stats["missing_timestamp"]` instead of
    printing per-row (the caller warns once with the aggregate count).
    """
    try:
        req_data = json.loads(row["data"])
        if not isinstance(req_data, dict):
            print(f"  Skipping row {row['id']}: data is not an object")
            return None
    except (json.JSONDecodeError, TypeError):
        print(f"  Skipping row {row['id']}: malformed data column")
        return None

    tokens = req_data.get("tokens") or {}
    if not isinstance(tokens, dict):
        print(f"  Skipping row {row['id']}: tokens is not an object")
        return None

    input_tokens, cached, completion, total = compute_token_buckets(tokens)
    prompt = safe_int(tokens.get("prompt_tokens", 0))

    if prompt == 0 and completion == 0:
        return None

    ts_ms = parse_iso_timestamp(row["timestamp"])
    if ts_ms is None:
        if stats is not None:
            stats["missing_timestamp"] = stats.get("missing_timestamp", 0) + 1
        return None

    # Fall back through null/empty req_data.model, then row["model"],
    # before giving up on "unknown". `dict.get("model", default)` does NOT
    # apply `default` when the key is present with an explicit null, so
    # that case must be handled separately.
    model = req_data.get("model") or row["model"] or "unknown"
    # Derive provider from first path segment for qualified IDs
    # (e.g. "deepseek-ai/deepseek-v4-flash" → "deepseek-ai"),
    # otherwise pass through the DB value.
    provider = row["provider"] or None
    if not provider and "/" in model:
        provider = model.split("/", 1)[0].lstrip("@")
    # Use local timezone (not UTC) so bridge file dates align with
    # tokscale --today / --since/--until, which use chrono::Local.
    date_str = datetime.fromtimestamp(ts_ms / 1000, tz=timezone.utc).astimezone().strftime("%Y-%m-%d")

    msg = {
        "role": "assistant",
        "model": model,
        "source": "9router",
        "timestamp": ts_ms,
        "usage": {
            "input": input_tokens,
            "output": completion,
            "cacheRead": cached,
            "cacheWrite": 0,
            "totalTokens": total,
        },
    }
    # Paid models omit usage.cost so tokscale reprices from tokens + pricing
    # data. Free variants embed an authoritative $0.00: the pricing lookup
    # strips "-free", so omitting cost would bill them at the paid rate.
    if is_free_model(model):
        msg["usage"]["cost"] = {"total": 0.0}
    if provider:
        msg["provider"] = provider
        msg["api"] = provider

    return {
        "date_str": date_str,
        "entry": {
            "type": "message",
            "id": row["id"],
            "message": msg,
        },
    }

def run():
    dbs = discover_router_dbs()
    if not dbs:
        print(f"9Router DB not found: {ROUTER_DB}")
        return

    ensure_bridge_dir()

    seen_ids: set[str] = set()
    messages_by_date: dict[str, list] = {}
    stats = {"missing_timestamp": 0}

    for db_path in dbs:
        conn = open_readonly_db(db_path)
        if conn is None:
            continue
        cursor = conn.execute(
            """
            SELECT id, timestamp, provider, model, connectionId, data
            FROM requestDetails
            WHERE status = 'success'
            ORDER BY timestamp
            """
        )
        for row in cursor:
            if row["id"] in seen_ids:
                continue
            # Row IDs are only added to seen_ids AFTER a successful
            # conversion, so an invalid row in the current DB doesn't
            # suppress a valid copy of the same ID from an older backup.
            result = convert_row_to_entry(row, stats)
            if result is None:
                continue
            seen_ids.add(row["id"])
            messages_by_date.setdefault(result["date_str"], []).append(result["entry"])
        conn.close()

    if stats["missing_timestamp"]:
        print(
            f"Warning: skipped {stats['missing_timestamp']} row(s) with "
            "missing/unparseable timestamps"
        )

    total_entries = 0
    for date_str, entries in sorted(messages_by_date.items()):
        filepath = BRIDGE_DIR / f"9router-{date_str}.jsonl"
        tmppath = None
        try:
            with tempfile.NamedTemporaryFile(mode="w", dir=str(BRIDGE_DIR), delete=False, suffix=".tmp") as tmp:
                tmppath = tmp.name
                session_header = {
                    "type": "session",
                    "id": f"9router-{date_str}",
                    "timestamp": datetime.fromtimestamp(
                        entries[0]["message"]["timestamp"] / 1000,
                        tz=timezone.utc,
                    ).isoformat(),
                    "cwd": "/"
                }
                tmp.write(json.dumps(session_header) + "\n")
                for entry in entries:
                    tmp.write(json.dumps(entry) + "\n")
                    total_entries += 1
            os.replace(tmppath, filepath)
        except Exception:
            if tmppath and os.path.exists(tmppath):
                os.unlink(tmppath)
            raise

    print(f"Bridge files written to: {BRIDGE_DIR}")
    print(f"Files: {len(messages_by_date)}, Messages: {total_entries}")
    print()
    print("Add this to ~/.config/tokscale/settings.json:")
    print(
        json.dumps(
            {
                "scanner": {
                    "extraScanPaths": {
                        "gjc": [str(BRIDGE_DIR)]
                    }
                }
            },
            indent=2,
        )
    )
    print()
    print("Then run: tokscale graph --client 9router")

if __name__ == "__main__":
    run()
