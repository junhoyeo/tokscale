#!/usr/bin/env python3
"""Bridge 9Router usage into tokscale via gjc-format JSONL files.

Reads 9Router request details from ~/.9router/db/data.sqlite and writes
JSONL files that tokscale's gjc client parser can consume.

Usage:
    python3 scripts/9router_tokscale_bridge_gjc.py

Then add to ~/.config/tokscale/settings.json:
    {"scanner": {"extraScanPaths": {"gjc": ["/home/USER/.local/share/9router-tokscale/sessions"]}}}

CRITICAL: Do NOT emit usage.cost.total in the JSONL output. The gjc parser
treats any present cost.total (even 0.0) as CostSource::ProviderReported,
which prevents tokscale from repricing via its pricing database. Omitting
the cost field lets tokscale reprice from tokens + pricing data.

See docs/9router-bridge.md for full documentation.
"""

import json
import os
import sqlite3
from pathlib import Path
from datetime import datetime, timezone

ROUTER_DB = Path.home() / ".9router" / "db" / "data.sqlite"
BRIDGE_DIR = Path.home() / ".local" / "share" / "9router-tokscale" / "sessions"


def ensure_bridge_dir():
    """Create output directory. Existing files are NOT deleted — each date's
    file is overwritten individually by the write loop, preserving historical
    data from previous bridge runs."""
    BRIDGE_DIR.mkdir(parents=True, exist_ok=True)


def parse_iso_timestamp(ts: str) -> int:
    """Convert ISO-8601 timestamp to Unix milliseconds."""
    try:
        dt = datetime.fromisoformat(ts.replace("Z", "+00:00"))
        return int(dt.timestamp() * 1000)
    except Exception:
        return int(datetime.now(timezone.utc).timestamp() * 1000)




def run():
    if not ROUTER_DB.exists():
        print(f"9Router DB not found: {ROUTER_DB}")
        return

    ensure_bridge_dir()
    router_conn = sqlite3.connect(ROUTER_DB)
    router_conn.row_factory = sqlite3.Row

    cursor = router_conn.execute(
        """
        SELECT
            id,
            timestamp,
            provider,
            model,
            connectionId,
            data
        FROM requestDetails
        WHERE status = 'success'
        ORDER BY timestamp
        """
    )

    messages_by_date: dict[str, list] = {}
    for row in cursor:
        req_data = json.loads(row["data"])
        tokens = req_data.get("tokens", {})

        prompt = tokens.get("prompt_tokens", 0) or 0
        completion = tokens.get("completion_tokens", 0) or 0
        cached = tokens.get("cached_tokens", 0) or 0

        reasoning = 0
        completion_details = tokens.get("completion_tokens_details", {})
        if completion_details:
            reasoning = completion_details.get("reasoning_tokens", 0) or 0
        completion += reasoning

        if prompt == 0 and completion == 0:
            continue

        ts_ms = parse_iso_timestamp(row["timestamp"])
        model = req_data.get("model", row["model"]) or "unknown"
        provider = row["provider"] or None
        date_str = datetime.fromtimestamp(ts_ms / 1000, tz=timezone.utc).strftime("%Y-%m-%d")

        total = prompt + completion

        msg = {
            "role": "assistant",
            "model": model,
            "source": "9router",
            "timestamp": ts_ms,
            "usage": {
                "input": prompt,
                "output": completion,
                "cacheRead": cached,
                "cacheWrite": 0,
                "totalTokens": total,
            },
        }
        if provider:
            msg["provider"] = provider
            msg["api"] = provider

        entry = {
            "type": "message",
            "message": msg,
        }

        messages_by_date.setdefault(date_str, []).append(entry)

    router_conn.close()

    total_entries = 0
    for date_str, entries in sorted(messages_by_date.items()):
        filepath = BRIDGE_DIR / f"9router-{date_str}.jsonl"
        with open(filepath, "w") as f:
            session_header = {
                "type": "session",
                "id": f"9router-{date_str}",
                "timestamp": entries[0]["message"]["timestamp"],
                "cwd": "/"
            }
            f.write(json.dumps(session_header) + "\n")
            for entry in entries:
                f.write(json.dumps(entry) + "\n")
                total_entries += 1

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
    print("Then run: tokscale graph --client gjc")


if __name__ == "__main__":
    run()
