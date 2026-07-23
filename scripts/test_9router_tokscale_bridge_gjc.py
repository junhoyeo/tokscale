#!/usr/bin/env python3
"""Tests for scripts/9router_tokscale_bridge_gjc.py.

Run with: python3 -m pytest scripts/test_9router_tokscale_bridge_gjc.py

The bridge module's filename starts with a digit ("9router_..."), which is
not a valid Python identifier, so it can't be imported with a plain
`import` statement. It is loaded here via importlib.util instead.
"""

import importlib.util
import json
import os
import sqlite3
import sys
import time
from pathlib import Path

import pytest

MODULE_PATH = Path(__file__).parent / "9router_tokscale_bridge_gjc.py"
spec = importlib.util.spec_from_file_location("bridge_9router", MODULE_PATH)
bridge = importlib.util.module_from_spec(spec)
sys.modules["bridge_9router"] = bridge
spec.loader.exec_module(bridge)


def make_row(**overrides):
    """Build a dict-based fake DB row with sensible defaults.

    `convert_row_to_entry` only ever accesses rows via `row["col"]`
    (bracket access), which a plain dict supports the same way
    `sqlite3.Row` does, so dicts are a valid stand-in in tests.
    """
    row = {
        "id": "req_1",
        "timestamp": "2026-01-15T12:00:00Z",
        "provider": "openai",
        "model": "gpt-4o",
        "connectionId": "conn_1",
        "data": json.dumps(
            {
                "model": "gpt-4o",
                "tokens": {
                    "prompt_tokens": 100,
                    "completion_tokens": 50,
                    "cached_tokens": 20,
                },
            }
        ),
    }
    row.update(overrides)
    return row


# ── compute_token_buckets: OpenAI prompt-includes-cached split ─────────────


def test_compute_token_buckets_splits_prompt_and_cached():
    # prompt_tokens (100) already includes cached_tokens (20), so the
    # non-cached input bucket is prompt - cached = 80, and total sums the
    # three non-overlapping buckets back up.
    input_tokens, cached, completion, total = bridge.compute_token_buckets(
        {"prompt_tokens": 100, "completion_tokens": 50, "cached_tokens": 20}
    )
    assert input_tokens == 80
    assert cached == 20
    assert completion == 50
    assert total == input_tokens + cached + completion
    assert total == 150


def test_compute_token_buckets_no_cache():
    input_tokens, cached, completion, total = bridge.compute_token_buckets(
        {"prompt_tokens": 100, "completion_tokens": 50}
    )
    assert input_tokens == 100
    assert cached == 0
    assert completion == 50
    assert total == 150


def test_compute_token_buckets_cached_greater_than_prompt_clamps_to_zero():
    # Malformed/unexpected upstream data where cached_tokens exceeds
    # prompt_tokens must not go negative.
    input_tokens, cached, completion, total = bridge.compute_token_buckets(
        {"prompt_tokens": 10, "completion_tokens": 5, "cached_tokens": 999}
    )
    assert input_tokens == 0
    assert cached == 999
    assert completion == 5
    assert total == 0 + 999 + 5


# ── safe_int / string & negative token scalars ──────────────────────────────


def test_safe_int_coerces_numeric_string():
    assert bridge.safe_int("100") == 100


def test_safe_int_non_numeric_string_becomes_zero():
    assert bridge.safe_int("bogus") == 0


def test_safe_int_none_becomes_zero():
    assert bridge.safe_int(None) == 0


def test_safe_int_clamps_negative_to_zero():
    assert bridge.safe_int(-5) == 0


def test_compute_token_buckets_coerces_string_prompt_tokens():
    # A stringified "prompt_tokens" (e.g. `"100"`) must not raise a
    # TypeError and abort the whole run — it should coerce like the int.
    input_tokens, cached, completion, total = bridge.compute_token_buckets(
        {"prompt_tokens": "100", "completion_tokens": "50", "cached_tokens": "20"}
    )
    assert input_tokens == 80
    assert cached == 20
    assert completion == 50
    assert total == 150


def test_compute_token_buckets_non_numeric_string_treated_as_zero():
    input_tokens, cached, completion, total = bridge.compute_token_buckets(
        {"prompt_tokens": "not-a-number", "completion_tokens": 50, "cached_tokens": 0}
    )
    assert input_tokens == 0
    assert completion == 50
    assert total == 50


def test_compute_token_buckets_negative_cached_clamped_at_ingestion():
    # A negative cached_tokens must be clamped to 0 up front, not just
    # clamped indirectly via the prompt-cached subtraction, so it can't
    # inflate the computed input bucket.
    input_tokens, cached, completion, total = bridge.compute_token_buckets(
        {"prompt_tokens": 100, "completion_tokens": 50, "cached_tokens": -20}
    )
    assert cached == 0
    assert input_tokens == 100
    assert total == 150


# ── convert_row_to_entry: end-to-end token math via a full row ─────────────


def test_convert_row_to_entry_computes_non_overlapping_usage():
    row = make_row()
    result = bridge.convert_row_to_entry(row)
    assert result is not None
    usage = result["entry"]["message"]["usage"]
    assert usage["input"] == 80
    assert usage["cacheRead"] == 20
    assert usage["output"] == 50
    assert usage["totalTokens"] == 150


# ── malformed / NULL `data` JSON rows are skipped ───────────────────────────


def test_convert_row_to_entry_skips_null_data():
    row = make_row(data=None)
    assert bridge.convert_row_to_entry(row) is None


def test_convert_row_to_entry_skips_malformed_json_data():
    row = make_row(data="{not valid json")
    assert bridge.convert_row_to_entry(row) is None


def test_convert_row_to_entry_skips_non_dict_data():
    row = make_row(data=json.dumps([1, 2, 3]))
    assert bridge.convert_row_to_entry(row) is None


# ── non-dict `tokens` field is skipped ──────────────────────────────────────


def test_convert_row_to_entry_skips_non_dict_tokens():
    row = make_row(data=json.dumps({"model": "gpt-4o", "tokens": [1, 2, 3]}))
    assert bridge.convert_row_to_entry(row) is None


def test_convert_row_to_entry_skips_string_tokens():
    row = make_row(data=json.dumps({"model": "gpt-4o", "tokens": "bogus"}))
    assert bridge.convert_row_to_entry(row) is None


def test_convert_row_to_entry_skips_zero_prompt_and_completion():
    row = make_row(
        data=json.dumps(
            {
                "model": "gpt-4o",
                "tokens": {"prompt_tokens": 0, "completion_tokens": 0, "cached_tokens": 0},
            }
        )
    )
    assert bridge.convert_row_to_entry(row) is None


def test_convert_row_to_entry_handles_string_token_scalars_end_to_end():
    # A row whose token counts arrived as strings (e.g. `"prompt_tokens":
    # "100"`) must not raise and abort the run — it should convert exactly
    # like the equivalent int-typed row.
    row = make_row(
        data=json.dumps(
            {
                "model": "gpt-4o",
                "tokens": {
                    "prompt_tokens": "100",
                    "completion_tokens": "50",
                    "cached_tokens": "20",
                },
            }
        )
    )
    result = bridge.convert_row_to_entry(row)
    assert result is not None
    usage = result["entry"]["message"]["usage"]
    assert usage["input"] == 80
    assert usage["cacheRead"] == 20
    assert usage["output"] == 50


# ── missing/unparseable timestamps are skipped, never default to now() ─────


def test_convert_row_to_entry_skips_missing_timestamp():
    row = make_row(timestamp=None)
    assert bridge.convert_row_to_entry(row) is None


def test_convert_row_to_entry_skips_empty_timestamp():
    row = make_row(timestamp="")
    assert bridge.convert_row_to_entry(row) is None


def test_convert_row_to_entry_skips_unparseable_timestamp():
    row = make_row(timestamp="not-a-timestamp")
    assert bridge.convert_row_to_entry(row) is None


def test_convert_row_to_entry_counts_missing_timestamp_in_stats():
    # The bridge counts skipped-for-timestamp rows and warns once with the
    # aggregate, rather than defaulting to datetime.now() (which would give
    # the same malformed row a new dedup key every day it's re-read from a
    # lingering backup DB, causing perpetual re-duplication).
    stats = {"missing_timestamp": 0}
    row = make_row(timestamp="garbage")
    assert bridge.convert_row_to_entry(row, stats) is None
    assert stats["missing_timestamp"] == 1

    assert bridge.convert_row_to_entry(make_row(timestamp=None), stats) is None
    assert stats["missing_timestamp"] == 2


def test_convert_row_to_entry_never_defaults_timestamp_to_now():
    # Directly pin down the regression: a malformed timestamp must not
    # silently become datetime.now().
    row = make_row(timestamp="not-a-timestamp")
    assert bridge.parse_iso_timestamp(row["timestamp"]) is None


# ── model fallback: null data.model falls through to row["model"] ──────────


def test_convert_row_to_entry_falls_back_to_row_model_when_data_model_is_null():
    # `req_data.get("model")` is explicitly None (not just absent) — must
    # still fall back to row["model"] before giving up on "unknown".
    row = make_row(
        model="gpt-4o-from-row",
        data=json.dumps(
            {
                "model": None,
                "tokens": {"prompt_tokens": 10, "completion_tokens": 5, "cached_tokens": 0},
            }
        ),
    )
    result = bridge.convert_row_to_entry(row)
    assert result is not None
    assert result["entry"]["message"]["model"] == "gpt-4o-from-row"


def test_convert_row_to_entry_unknown_when_both_model_sources_missing():
    row = make_row(
        model=None,
        data=json.dumps(
            {
                "model": None,
                "tokens": {"prompt_tokens": 10, "completion_tokens": 5, "cached_tokens": 0},
            }
        ),
    )
    result = bridge.convert_row_to_entry(row)
    assert result is not None
    assert result["entry"]["message"]["model"] == "unknown"


# ── local-date bucketing ────────────────────────────────────────────────────


def test_convert_row_to_entry_buckets_by_local_date():
    # ts_ms is derived from the ISO timestamp, then the bucket date is
    # computed via .astimezone() (local time), not UTC. Compare against the
    # same conversion tokscale's bridge performs so the test tracks the
    # implementation's documented local-date behavior rather than hardcoding
    # a timezone-dependent literal.
    from datetime import datetime, timezone

    row = make_row(timestamp="2026-01-15T23:30:00Z")
    result = bridge.convert_row_to_entry(row)
    assert result is not None

    ts_ms = result["entry"]["message"]["timestamp"]
    expected_date = (
        datetime.fromtimestamp(ts_ms / 1000, tz=timezone.utc)
        .astimezone()
        .strftime("%Y-%m-%d")
    )
    assert result["date_str"] == expected_date


def test_convert_row_to_entry_groups_same_utc_day_rows_into_one_bucket():
    # Pin TZ=UTC so the local-date bucket matches the UTC date exactly.
    # Without pinning, this assumption breaks under some host timezones —
    # e.g. TZ=America/Noronha (UTC-2) shifts 01:00Z to local 2026-01-14
    # 23:00 while 02:00Z stays on 2026-01-15, landing the two rows in
    # different local-date buckets and failing this test nondeterministically
    # depending on the machine it runs on.
    original_tz = os.environ.get("TZ")
    os.environ["TZ"] = "UTC"
    if hasattr(time, "tzset"):
        time.tzset()
    try:
        row_a = make_row(id="req_a", timestamp="2026-01-15T01:00:00Z")
        row_b = make_row(id="req_b", timestamp="2026-01-15T02:00:00Z")
        result_a = bridge.convert_row_to_entry(row_a)
        result_b = bridge.convert_row_to_entry(row_b)
        assert result_a["date_str"] == result_b["date_str"]
    finally:
        if original_tz is None:
            os.environ.pop("TZ", None)
        else:
            os.environ["TZ"] = original_tz
        if hasattr(time, "tzset"):
            time.tzset()


# ── source stamping ─────────────────────────────────────────────────────────


def test_convert_row_to_entry_stamps_9router_source():
    row = make_row()
    result = bridge.convert_row_to_entry(row)
    assert result["entry"]["message"]["source"] == "9router"


# ── free-variant cost embed: free models pin $0.00, paid models reprice ─────


@pytest.mark.parametrize(
    "model",
    [
        "kimi-k2.5-free",
        "openrouter/kimi-k2.5-free",
        "moonshotai/kimi-k2.5:free",
        "KIMI-K2.5-FREE",  # case-insensitive suffix match
        "gpt-oss-120b:FREE",
    ],
)
def test_convert_row_to_entry_free_model_embeds_zero_cost(model):
    # Tokscale's pricing lookup strips the "-free" suffix, so an omitted
    # cost would reprice the free variant at the PAID base-model rate.
    # Free rows must carry an authoritative $0.00 (cost.total present ⇒
    # CostSource::ProviderReported ⇒ no repricing), tokens still counted.
    row = make_row(model=model, data=json.dumps({
        "model": model,
        "tokens": {"prompt_tokens": 100, "completion_tokens": 50},
    }))
    result = bridge.convert_row_to_entry(row)
    assert result is not None
    usage = result["entry"]["message"]["usage"]
    assert usage["cost"] == {"total": 0.0}
    assert usage["totalTokens"] == 150


@pytest.mark.parametrize(
    "model",
    [
        "gpt-4o",
        "kimi-k2.5",  # paid base model of a free variant
        "freedom-model",  # "free" substring but not a suffix
        "gpt-4o-freestyle",
    ],
)
def test_convert_row_to_entry_paid_model_omits_cost(model):
    # Paid rows must NOT carry usage.cost — any present cost.total (even
    # 0.0) is authoritative and would block repricing from tokens.
    row = make_row(model=model, data=json.dumps({
        "model": model,
        "tokens": {"prompt_tokens": 100, "completion_tokens": 50},
    }))
    result = bridge.convert_row_to_entry(row)
    assert result is not None
    assert "cost" not in result["entry"]["message"]["usage"]


def test_is_free_model_suffix_rules():
    assert bridge.is_free_model("kimi-k2.5-free")
    assert bridge.is_free_model("gpt-oss-120b:free")
    assert bridge.is_free_model("Kimi-K2.5-Free")
    assert not bridge.is_free_model("kimi-k2.5")
    assert not bridge.is_free_model("free-model")
    assert not bridge.is_free_model("model-freeform")


# ── atomic write cleans up its .tmp file on failure ─────────────────────────


def test_run_cleans_up_tmp_file_when_write_fails(tmp_path, monkeypatch):
    db_path = tmp_path / "data.sqlite"
    conn = sqlite3.connect(db_path)
    conn.execute(
        "CREATE TABLE requestDetails (id TEXT, timestamp TEXT, provider TEXT, "
        "model TEXT, connectionId TEXT, data TEXT, status TEXT)"
    )
    conn.execute(
        "INSERT INTO requestDetails VALUES (?, ?, ?, ?, ?, ?, ?)",
        (
            "req_1",
            "2026-01-15T12:00:00Z",
            "openai",
            "gpt-4o",
            "conn_1",
            json.dumps(
                {
                    "model": "gpt-4o",
                    "tokens": {"prompt_tokens": 10, "completion_tokens": 5, "cached_tokens": 0},
                }
            ),
            "success",
        ),
    )
    conn.commit()
    conn.close()

    bridge_dir = tmp_path / "sessions"
    monkeypatch.setattr(bridge, "ROUTER_DB", db_path)
    monkeypatch.setattr(bridge, "BRIDGE_DIR", bridge_dir)

    original_dumps = bridge.json.dumps
    call_count = {"n": 0}

    def flaky_dumps(*args, **kwargs):
        call_count["n"] += 1
        # Let the session header line through, then blow up on the first
        # message line so the write fails mid-file, after the .tmp file
        # was created but before os.replace() runs.
        if call_count["n"] == 2:
            raise RuntimeError("boom")
        return original_dumps(*args, **kwargs)

    monkeypatch.setattr(bridge.json, "dumps", flaky_dumps)

    try:
        bridge.run()
        assert False, "expected run() to propagate the write failure"
    except RuntimeError:
        pass

    leftover_tmp_files = list(bridge_dir.glob("*.tmp"))
    assert leftover_tmp_files == [], f"leaked .tmp files: {leftover_tmp_files}"


# ── row IDs are only marked seen after a successful conversion ─────────────


def _insert_request_row(db_path, **row):
    conn = sqlite3.connect(db_path)
    conn.execute(
        "CREATE TABLE requestDetails (id TEXT, timestamp TEXT, provider TEXT, "
        "model TEXT, connectionId TEXT, data TEXT, status TEXT)"
    )
    conn.execute(
        "INSERT INTO requestDetails VALUES (?, ?, ?, ?, ?, ?, ?)",
        (
            row["id"],
            row["timestamp"],
            row.get("provider", "openai"),
            row.get("model", "gpt-4o"),
            row.get("connectionId", "conn_1"),
            row["data"],
            row.get("status", "success"),
        ),
    )
    conn.commit()
    conn.close()


def test_run_uses_backup_row_when_current_db_row_is_invalid(tmp_path, monkeypatch):
    # A bad row in the current DB must not suppress a valid copy of the
    # same ID from an older backup — seen_ids should only gain an entry
    # once a row actually converts successfully.
    router_dir = tmp_path / "router"
    router_dir.mkdir()
    current_db = router_dir / "data.sqlite"
    backups_dir = router_dir / "backups" / "v1"
    backups_dir.mkdir(parents=True)
    backup_db = backups_dir / "data.sqlite"

    valid_data = json.dumps(
        {
            "model": "gpt-4o",
            "tokens": {"prompt_tokens": 10, "completion_tokens": 5, "cached_tokens": 0},
        }
    )

    # Current DB: malformed `data` column for req_1 -> conversion fails.
    _insert_request_row(
        current_db, id="req_1", timestamp="2026-01-15T12:00:00Z", data=None
    )
    # Backup DB: same ID, valid data -> should still make it through.
    _insert_request_row(
        backup_db, id="req_1", timestamp="2026-01-15T12:00:00Z", data=valid_data
    )

    bridge_dir = tmp_path / "sessions"
    monkeypatch.setattr(bridge, "ROUTER_DB", current_db)
    monkeypatch.setattr(bridge, "BRIDGE_DIR", bridge_dir)

    bridge.run()

    output_file = bridge_dir / "9router-2026-01-15.jsonl"
    assert output_file.exists()
    lines = output_file.read_text().splitlines()
    message_lines = [json.loads(line) for line in lines if json.loads(line).get("type") == "message"]
    assert len(message_lines) == 1
    assert message_lines[0]["id"] == "req_1"
    assert message_lines[0]["message"]["usage"]["input"] == 10


# ── read-only DB access ─────────────────────────────────────────────────────


def test_open_readonly_db_missing_file_returns_none_and_creates_nothing(tmp_path):
    missing_path = tmp_path / "does-not-exist" / "data.sqlite"
    assert bridge.open_readonly_db(missing_path) is None
    assert not missing_path.exists()
    assert not missing_path.parent.exists()


def test_open_readonly_db_does_not_write_to_db(tmp_path):
    db_path = tmp_path / "data.sqlite"
    _insert_request_row(
        db_path,
        id="req_1",
        timestamp="2026-01-15T12:00:00Z",
        data=json.dumps({"model": "gpt-4o", "tokens": {"prompt_tokens": 1}}),
    )

    conn = bridge.open_readonly_db(db_path)
    assert conn is not None
    try:
        with pytest.raises(sqlite3.OperationalError):
            conn.execute(
                "INSERT INTO requestDetails VALUES (?, ?, ?, ?, ?, ?, ?)",
                ("req_2", "2026-01-15T12:00:00Z", "openai", "gpt-4o", "conn_1", "{}", "success"),
            )
            conn.commit()
    finally:
        conn.close()
