#!/usr/bin/env bash
# Red-path tests for scripts/check-dsh-cache-migration.sh.
#
# The gate itself needs two published binaries and a release build to say
# anything, which makes the ways it is supposed to fail expensive to check and
# easy to lose. This drives it with stub binaries and a synthetic fixture
# instead, so every red path runs in seconds without npm or cargo.
#
# The stub models the fates a shard meets in `read_shard_with_limit` and
# `ensure_namespace_loaded`. A stored parser version that matches the stub's is
# LOADED: the report it saved when it last parsed comes back verbatim. One that
# does not match is STALE: parsed again and rewritten. A stub told which
# older cache format it migrates serves a shard in that format and re-persists
# it afterwards, MIGRATED -- the rows are served, and the bytes still move. One
# told which older format it finds undecodable warns on stderr the way
# `warn_cache_failure_once` does and parses again, INVALID. Nothing stored is a
# cold parse. Serve, migrate and discard are three outcomes and only the middle
# one both answers from the cache and moves the bytes, which is why the gate
# grades the canary and not the bytes.
#
# The canary is the gate's own: it plants a transcript in its copy of the
# fixture and edits its usage figure after A and C have cached it. The stub
# parses that transcript for real, so a stub that serves reports the figure it
# cached and a stub that parses again reports the edit -- and one stub, told
# its fingerprint sees the edit, reparses the canary alone, which is what the
# gate's control leg has to turn into a notice rather than a red.
#
# That is enough to stage a predecessor that persisted nothing, a pin that names
# the wrong release, a build whose parser version was never bumped past its
# predecessor's, stale model attribution behind unchanged token totals, an
# unpinned leg that discards a cache it owns instead of serving it, one that
# discards a cache one format back where a migration was due, one whose wire
# migration serves the rows, one whose migrated rows are never written back,
# one whose migration fails to decode, and one whose fingerprint sees the
# canary.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="${ROOT_DIR}/scripts/check-dsh-cache-migration.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

failures=0

python3 - "${TMP_DIR}" <<'PY'
import json
import pathlib
import stat
import sys

tmp = pathlib.Path(sys.argv[1])
fixtures = tmp / "fixtures"
fixture = fixtures / "stub-migration"
bins = tmp / "bin"
reports = tmp / "reports"
for directory in (bins, reports):
    directory.mkdir(parents=True, exist_ok=True)

# Two transcripts, because the gate requires an entry per transcript and one
# would not tell a per-file check from a "some cache exists" check.
for name in ("session-one", "session-two"):
    path = fixture / "sessions" / "--fixture--" / name / "session.jsonl"
    path.parent.mkdir(parents=True, exist_ok=True)
    # The stub never reads these; the gate lists them and looks for their paths
    # in the cache payload. The canary the gate plants beside them is the one
    # transcript the stub does parse.
    path.write_text('{"type":"session","id":"%s"}\n' % name)

BUCKETS = ("input", "output", "cacheRead", "cacheWrite", "reasoning", "messageCount")


def report(models):
    """A `tokscale --json` document with the fields the comparator reads."""
    entries = [
        dict(zip(BUCKETS, buckets), client="dsh", provider="p", model=model, cost=0.0)
        for model, buckets in sorted(models.items())
    ]
    return {
        "groupBy": "client,model",
        "entries": entries,
        "totalInput": sum(entry["input"] for entry in entries),
        "totalOutput": sum(entry["output"] for entry in entries),
        "totalCacheRead": sum(entry["cacheRead"] for entry in entries),
        "totalCacheWrite": sum(entry["cacheWrite"] for entry in entries),
        "totalMessages": sum(entry["messageCount"] for entry in entries),
        "totalCost": 0.0,
    }


def expectation(doc):
    return {
        "totals": {
            field: doc[field]
            for field in (
                "totalInput",
                "totalOutput",
                "totalCacheRead",
                "totalCacheWrite",
                "totalMessages",
            )
        },
        "models": {
            f"{entry['client']}/{entry['provider']}/{entry['model']}": {
                bucket: entry[bucket] for bucket in BUCKETS
            }
            for entry in doc["entries"]
        },
    }


#                        input out cacheRead cacheWrite reasoning messages
CURRENT = report({"served": (100, 10, 1000, 2, 0, 1), "plain": (200, 20, 2000, 4, 0, 1)})
# What a predecessor that drops a call reports: fewer tokens, fewer messages.
BASELINE = report({"plain": (200, 20, 2000, 4, 0, 1)})
# What a predecessor with stale model attribution reports: the same totals as
# CURRENT to the token, credited to a model CURRENT does not name at all. Token
# buckets cannot tell this from CURRENT; only the per-model split can.
ATTRIBUTION = report({"requested": (300, 30, 3000, 6, 0, 2)})
assert {k: v for k, v in ATTRIBUTION.items() if k.startswith("total")} == {
    k: v for k, v in CURRENT.items() if k.startswith("total")
}, "the attribution case only tests anything while its totals match CURRENT"

for name, doc in (("current", CURRENT), ("baseline", BASELINE), ("attribution", ATTRIBUTION)):
    (reports / f"{name}.json").write_text(json.dumps(doc))

(fixture / "expected.json").write_text(
    json.dumps({"current": expectation(CURRENT), "predecessors": {"4": expectation(BASELINE)}})
)

# The stub's cache format, and the one before it. A real shard carries
# CACHE_FORMAT_VERSION in this byte, and `read_shard_with_limit` checks it only
# *after* the parser identity -- so a shard this build owns by identity can
# still be in an older format, which it migrates on the wire, discards, or
# fails to decode.
STUB_CACHE_FORMAT = 7
STUB_LEGACY_CACHE_FORMAT = 6

STUB = r'''
"""Stub tokscale: one Python program, one JSON configuration per stub binary.

Reads the cache the way the gate reasons about it. What it serves is the
report it saved when it last parsed, which carries the canary's figure of that
moment; what it parses is its configured cold report plus the canary
transcript as it reads right now.
"""
import json
import os
import pathlib
import sys

config = json.loads(pathlib.Path(sys.argv[1]).read_text())
if sys.argv[2:] == ["--version"]:
    print(f"tokscale {config['label']}")
    sys.exit(0)

home = pathlib.Path(os.environ["HOME"])
dsh_home = pathlib.Path(os.environ["DSH_HOME"])
cache = home / ".config" / "tokscale" / "cache"
shards = cache / "source-message-cache-v2" / "dsh"
served_report = cache / "stub-served-report.json"
writer = cache / "stub-writer"
transcripts = sorted((dsh_home / "sessions").rglob("session.jsonl"))
canary = dsh_home / "sessions" / "--fixture--" / "session-canary" / "session.jsonl"
CANARY_MODEL = "fixture-canary-model"
TOTALS = {
    "totalInput": "input",
    "totalOutput": "output",
    "totalCacheRead": "cacheRead",
    "totalCacheWrite": "cacheWrite",
    "totalMessages": "messageCount",
}


def canary_row():
    """What a parse of the canary transcript yields right now."""
    if not canary.exists():
        return None
    for line in canary.read_text().splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("type") == "assistant/message":
            usage = event["data"]["usage"]
            return {
                "input": usage["inputTokens"],
                "output": usage["outputTokens"],
                "cacheRead": usage["cacheReadTokens"],
                "cacheWrite": usage["cacheWriteTokens"],
                "reasoning": 0,
                "messageCount": 1,
                "client": "dsh",
                "provider": "fixture-provider",
                "model": CANARY_MODEL,
                "cost": 0.0,
            }
    return None


def compose(entries):
    doc = {"groupBy": "client,model", "entries": entries, "totalCost": 0.0}
    for field, bucket in TOTALS.items():
        doc[field] = sum(entry[bucket] for entry in entries)
    return doc


def parsed():
    entries = list(json.loads(pathlib.Path(config["cold_report"]).read_text())["entries"])
    row = canary_row()
    if row is not None:
        entries.append(row)
    return compose(entries)


def varint(value):
    """bincode `DefaultOptions`: <251 inline, then 251/252/253 = u16/u32/u64."""
    if value < 251:
        return bytes([value])
    if value < 1 << 16:
        return b"\xfb" + value.to_bytes(2, "little")
    if value < 1 << 32:
        return b"\xfc" + value.to_bytes(4, "little")
    return b"\xfd" + value.to_bytes(8, "little")


def envelope(payload):
    """A `CachedShardEnvelope` header the gate can read, over a payload it only
    probes for transcript paths and `dsh:` dedup keys."""
    return (
        bytes([config["format"]])
        + varint(3)
        + b"dsh"
        + bytes([config["writes"]])
        + varint(len(payload))
        + payload
    )


def entry_bytes(path, key):
    return b"\x00".join([str(path).encode(), key.encode(), b""])


def write_shards(suffix=b""):
    """One shard for the fixture's own transcripts, one for the canary, so the
    gate can leave the canary's shard out of a byte comparison on its own."""
    shards.mkdir(parents=True, exist_ok=True)
    rows = [path for path in transcripts if path != canary]
    (shards / "shard-00.bin").write_bytes(
        envelope(b"".join(entry_bytes(p, f"dsh:msg:{i}:stub") for i, p in enumerate(rows)) + suffix)
    )
    if canary in transcripts:
        (shards / "shard-01.bin").write_bytes(
            envelope(entry_bytes(canary, "dsh:msg:canary:stub") + suffix)
        )


stored = None
if (shards / "shard-00.bin").exists():
    header = (shards / "shard-00.bin").read_bytes()
    # Format byte first; the parser version follows the 3-byte namespace and
    # its length byte.
    stored = (header[0], header[5])

fate = "cold"
if stored is not None:
    stored_format, stored_parser = stored
    if stored_parser != config.get("serves") or not served_report.exists():
        fate = "stale"
    elif stored_format == config.get("invalid_format"):
        fate = "invalid"
    elif stored_format == config.get("stale_format"):
        fate = "stale"
    elif config.get("rejects_foreign") and writer.read_text() != config["label"]:
        # Owns the identity and throws the rows away anyway: what entries whose
        # stored key no longer matches this build's look like. Its own cache
        # it keeps, so the gate's control leg still serves.
        fate = "stale"
    elif stored_format == config.get("migrates_format"):
        fate = "migrated"
    else:
        fate = "loaded"

# Each shard meets its own fate. The canary lives in shard-01; a build that
# serves shard-00 and discards shard-01 parses only the canary again, and the
# served report must show that row fresh while everything else is served.
# `discards_canary_shard` is gated like `rejects_foreign`, on a cache another
# stub wrote: this build keeps its own shards, so the gate's control leg D
# still serves the canary and the canary stays usable as evidence against B.
canary_shard = shards / "shard-01.bin"
canary_discarded = False
if fate in ("loaded", "migrated") and canary_shard.exists():
    canary_format = canary_shard.read_bytes()[0]
    if config.get("discards_canary_shard") and writer.read_text() != config["label"]:
        canary_discarded = True
    elif canary_format == config.get("stale_format") or canary_format == config.get("invalid_format"):
        canary_discarded = True

if fate in ("loaded", "migrated"):
    doc = json.loads(served_report.read_text())
    if canary_discarded:
        fresh = canary_row()
        entries = [entry for entry in doc["entries"] if entry["model"] != CANARY_MODEL]
        if fresh is not None:
            entries.append(fresh)
        doc = compose(entries)
        # A discard-and-reparse rewrites the shard it threw away.
        canary_shard.write_bytes(
            envelope(entry_bytes(canary, "dsh:msg:canary:stub") + b"\x02")
        )
    if config.get("sees_edit"):
        # A fingerprint that catches the in-place edit: the canary row alone is
        # parsed again and its shard rewritten, everything else is served.
        fresh = canary_row()
        for entry in doc["entries"]:
            if entry["model"] == CANARY_MODEL and fresh is not None and entry["input"] != fresh["input"]:
                entry.update(fresh)
                (shards / "shard-01.bin").write_bytes(
                    envelope(entry_bytes(canary, "dsh:msg:canary:stub") + b"\x01")
                )
        doc = compose(doc["entries"])
    print(json.dumps(doc))
    if fate == "migrated":
        write_shards()
    sys.exit(0)

if fate == "invalid":
    print(
        "tokscale: warning: source message cache shard is invalid "
        f"({shards / 'shard-00.bin'}): io error: unexpected end of file",
        file=sys.stderr,
    )

doc = parsed()
print(json.dumps(doc))
if config.get("writes") is not None:
    write_shards(config.get("payload_suffix", "").encode())
    served_report.write_text(json.dumps(doc))
    writer.write_text(config["label"])
'''
core = bins / "stub.py"
core.write_text(STUB)

# `serves` is the stored parser version a stub reads a cache back from, and
# `writes` the one it writes; `None` for `writes` is a build that persists
# nothing. `format` is the cache format it writes. `migrates_format` is an
# older format it deserializes, serves and re-persists; `stale_format` one it
# throws away without a word (a CACHE_FORMAT_VERSION bump with no legacy
# branch); `invalid_format` one whose wire migration fails to decode, which
# warns. `rejects_foreign` discards a cache another stub wrote while keeping its
# own; `sees_edit` is a fingerprint that catches the canary's in-place edit.
STUBS = {
    "this-v5-current": dict(serves=5, writes=5, cold="current"),
    "this-v5-discards": dict(serves=5, writes=5, cold="current", rejects_foreign=True, payload_suffix="\x00"),
    "this-v5-discards-format": dict(serves=5, writes=5, cold="current", stale_format=STUB_LEGACY_CACHE_FORMAT),
    "this-v5-migrates": dict(serves=5, writes=5, cold="current", migrates_format=STUB_LEGACY_CACHE_FORMAT),
    "this-v5-invalid6": dict(serves=5, writes=5, cold="current", invalid_format=STUB_LEGACY_CACHE_FORMAT),
    "this-v5-hashes": dict(serves=5, writes=5, cold="current", sees_edit=True),
    # Serves shard-00 and throws shard-01 -- the canary's own shard -- away.
    "this-v5-discards-canary": dict(serves=5, writes=5, cold="current", discards_canary_shard=True),
    "this-v4-current": dict(serves=4, writes=4, cold="current"),
    "prev-v5-current": dict(serves=5, writes=5, cold="current"),
    "prev-v5-format6": dict(serves=5, writes=5, cold="current", format=STUB_LEGACY_CACHE_FORMAT),
    "prev-v4-baseline": dict(serves=4, writes=4, cold="baseline"),
    "prev-v3-baseline": dict(serves=3, writes=3, cold="baseline"),
    "prev-v4-attribution": dict(serves=4, writes=4, cold="attribution"),
    "prev-v4-nocache": dict(serves=4, writes=None, cold="baseline"),
}
for label, options in STUBS.items():
    settings = {"label": label, "format": STUB_CACHE_FORMAT, **options}
    settings["cold_report"] = str(reports / f"{settings.pop('cold')}.json")
    (bins / f"{label}.json").write_text(json.dumps(settings))
    path = bins / label
    # The gate runs every leg under `env -i` with a bare PATH, so the stub
    # names the interpreter this test is running under rather than searching.
    path.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{core}" "{bins / (label + ".json")}" "$@"\n')
    path.chmod(path.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
PY

case_number=0

# Runs the gate once and checks its exit code, then every phrase the case is
# about. Output is kept on failure so a broken expectation is readable.
expect() { # description expected-exit this prev pin [phrase...]
  local description="$1" expected_exit="$2" this="$3" prev="$4" pin="$5"
  shift 5
  case_number=$((case_number + 1))
  local log="${TMP_DIR}/case-${case_number}.log"
  local actual_exit=0
  DSH_GATE_FIXTURES="${TMP_DIR}/fixtures" DSH_GATE_OUT="${TMP_DIR}/out-${case_number}" \
    bash "${SCRIPT_UNDER_TEST}" \
    "${TMP_DIR}/bin/${this}" "${TMP_DIR}/bin/${prev}" stub-migration "${pin}" \
    > "${log}" 2>&1 || actual_exit=$?

  if [[ "${actual_exit}" != "${expected_exit}" ]]; then
    echo "FAIL  ${description}: exited ${actual_exit}, expected ${expected_exit}"
    sed 's/^/        /' "${log}"
    failures=$((failures + 1))
    return 0
  fi
  local phrase
  for phrase in "$@"; do
    if ! grep -qF -- "${phrase}" "${log}"; then
      echo "FAIL  ${description}: output never said '${phrase}'"
      sed 's/^/        /' "${log}"
      failures=$((failures + 1))
      return 0
    fi
  done
  echo "ok    ${description}"
}

# Scoped to the FAIL lines of the case that just ran, for the cases that are
# as much about what the gate must NOT have failed on as about what it did.
# The totals table names the same fields on every run, so the whole log cannot
# answer this; only the failure lines can.
refute_in_failure() { # description phrase
  local log="${TMP_DIR}/case-${case_number}.log"
  if grep '^FAIL' "${log}" | grep -qF -- "$2"; then
    echo "FAIL  $1: the failure named '$2'"
    sed 's/^/        /' "${log}"
    failures=$((failures + 1))
    return 0
  fi
  echo "ok    $1"
}

expect "a pinned predecessor whose cache migrates cleanly passes" \
  0 this-v5-current prev-v4-baseline 4 "PASS" \
  "parsed again, as a rejected identity requires"

expect "the last published release passes with only leg A ungraded" \
  0 this-v5-current prev-v5-current unpinned "PASS" \
  "printed and not graded" \
  "B vs C here is what fails on the next missing parser-version bump" \
  "the released rows were served, not parsed again"

# The unpinned leg only carries that load while leg B really serves the released
# rows. A build that owns them by identity and throws them away anyway turns B
# into a second cold parse that agrees with C for free, and the leg would
# otherwise go on passing. Both the canary and the bytes see this one: the
# shards already carried this build's format, so no rewrite was due.
expect "an unpinned leg that discards a cache it owns fails" \
  1 this-v5-discards prev-v5-current unpinned \
  "parsed the canary transcript again" \
  "did not leave its shards alone" \
  "can no longer fail on a missing parser-version bump"
refute_in_failure "  ...and not because any leg disagreed" "differs"

# A cache is many shards and `read_shard_with_limit` decides each one's fate on
# its own. A build that serves every shard but the one holding the canary
# parses only that transcript again -- one shard lost among many -- and the
# cache-wide picture still looks served. The canary is what sees it, and this
# is the case that keeps the stub honest about per-shard fates.
expect "an unpinned leg that discards one shard of a cache it owns fails" \
  1 this-v5-discards-canary prev-v5-current unpinned \
  "parsed the canary transcript again"
refute_in_failure "  ...and not because any leg disagreed" "differs"
# ...and the same discard one cache format back, where the bytes cannot see it.
# A CACHE_FORMAT_VERSION bump with no legacy branch for the released format
# reads every released shard as Stale, silently, and rewrites it in the new
# format -- exactly the bytes a wire migration would have left. Only the canary
# tells this from the migrating case below, and it has to.
expect "an unpinned leg that discards a cache one format back fails" \
  1 this-v5-discards-format prev-v5-format6 unpinned \
  "parsed the canary transcript again" \
  "cache format this build no longer migrates"
refute_in_failure "  ...and not because of the bytes, which were due to move" "did not leave its shards alone"
refute_in_failure "  ...and not because any leg disagreed" "differs"

# A CACHE_FORMAT_VERSION bump keeps a wire migration for the format before it
# -- three of them live in message_cache.rs today -- and that migration
# DESERIALIZES the released rows, serves them, and re-persists them in the new
# format (`ShardReadStatus::Migrated` -> `rewrite_shards` -> `dirty`). Nothing
# was discarded, leg B answered from the cache, and the shard bytes moved
# anyway. Format bumps are driven by whichever client changed its stored
# layout, usually not DSH, so this is the ordinary shape of the next such PR.
expect "an unpinned leg whose released cache migrates to a new format passes with a notice" \
  0 this-v5-migrates prev-v5-format6 unpinned "PASS" \
  "a rewrite of its shards was due for the format alone" \
  "the released rows were served, not parsed again"

# Skipping the byte comparison does not leave that leg with nothing to say. A
# build that reads the old-format rows, serves them and never re-persists them
# -- a wire migration returning `Loaded` instead of `Migrated`, or the same
# `save_if_dirty` warning-and-return that the no-cache case is about -- leaves
# the released shards sitting in the old format and migrates them again on
# every scan for the rest of the cache's life. `this-v5-current` serves a shard
# it owns by identity and writes nothing back, which is exactly that.
expect "an unpinned leg whose migrated cache is never re-persisted fails" \
  1 this-v5-current prev-v5-format6 unpinned \
  "after leg B the released cache has" \
  "cache format [6] rather than this build's [7]" \
  "every scan from here migrates them again"
refute_in_failure "  ...and not because any leg disagreed" "differs"

# A wire migration whose legacy struct no longer matches the released payload
# does not read as Stale; it errors, and `warn_cache_failure_once` says so on
# stderr while the CLI exits 0. The gate holds that stderr and must not throw
# it away.
expect "an unpinned leg whose wire migration fails to decode fails, and prints the warning" \
  1 this-v5-invalid6 prev-v5-format6 unpinned \
  "warned about the cache on stderr" \
  "source message cache shard is invalid" \
  "parsed the canary transcript again"

# The canary is only evidence while this build's fingerprint is blind to the
# edit. A build that sees it reparses the canary on its own cache too, which
# the control leg turns into a notice: the served-vs-reparsed check stands
# down and the byte comparison leaves the canary's rewritten shard out, so a
# fingerprint that merely sees more is not reported as a cache that was thrown
# away.
expect "a fingerprint that sees the canary edit degrades to a notice" \
  0 this-v5-hashes prev-v5-current unpinned "PASS" \
  "NOTE  D, this build warm on its own cache, reports the canary's edited figure" \
  "fingerprint sees an in-place edit"

# Control: the same leg going vacuous because the version was just bumped is not
# a failure -- no unreleased build can make `latest` carry the new identity --
# but it has to say so rather than report a migration it never ran.
expect "an unpinned leg whose release predates a fresh bump passes with a notice" \
  0 this-v5-current prev-v4-baseline unpinned "PASS" \
  "grades nothing about the identity path" \
  "parsed again, as a rejected identity requires"

expect "a predecessor that persisted no cache fails" \
  1 this-v5-current prev-v4-nocache 4 "persisted no DSH cache shards"

expect "a pin that names the wrong release fails" \
  1 this-v5-current prev-v3-baseline 4 "wrote DSH parser version(s) [3], expected 4"

# The bug the pin exists for: with the bump deleted, this build owns its
# predecessor's cache and serves the stale rows. The header says so, and so
# does the canary.
expect "a build still on its predecessor's parser version fails" \
  1 this-v4-current prev-v4-baseline 4 \
  "still on DSH parser version 4" \
  "served the canary row the release cached" \
  "served, not reparsed"

# The bug the per-model comparison exists for. Run unpinned so no parser-version
# assertion can fire and the only thing left to fail on is the split.
expect "stale model attribution behind unchanged totals fails" \
  1 this-v4-current prev-v4-attribution unpinned \
  "served, not reparsed" \
  "dsh/p/requested"
refute_in_failure "  ...and not because any token total moved" "total"
refute_in_failure "  ...and not because the canary was reparsed" "canary"

if ((failures > 0)); then
  echo "${failures} case(s) failed"
  exit 1
fi
echo "all cases passed"
