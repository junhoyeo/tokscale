#!/usr/bin/env bash
# A cache written by a published release, read once by this build, must report
# what a cold scan of this build reports -- per model, not only in total -- and
# it has to have been read, not thrown away and parsed over again.
#
#   scripts/check-dsh-cache-migration.sh \
#     <this-binary> <released-binary> <fixture> <released-parser-version|unpinned>
#
# One released predecessor per invocation, so each case is separately named and
# separately red. Runs both binaries over a copy of
# crates/tokscale-core/tests/fixtures/<fixture>, isolated HOME per leg, DSH_HOME
# pinned at the copy, network untouched except for the pricing fetch the CLI
# does on its own:
#
#   A  released, cold HOME
#   C  this,     cold HOME
#        (the canary transcript is edited here -- THE CANARY, below)
#   B  this,     warm on A's HOME    the migration leg
#   D  this,     warm on C's HOME    the control
#
# The fourth argument is the DSH parser version the released binary is expected
# to have written, or `unpinned` for the last published release, whose version
# is whatever it happens to be.
#
# PIN THE PREDECESSOR. A release that already carries this build's DSH parser
# version writes a cache this build accepts as its own, so leg B serves those
# rows unchanged and lands on the right figure without migrating anything --
# and would keep landing there if the version bump under test were deleted,
# because a rejected identity merely reparses. Only a predecessor from *before*
# the bump writes the stale rows the migration is supposed to retire, which is
# why the graded legs name a version instead of following `latest`.
#
# The unpinned leg is not the ungraded one. Only leg A's own report goes
# ungraded there, because a release that moves has no baseline to hold it to;
# B, C and D are compared exactly as they are on a pinned leg. And serving is
# the point of that leg rather than a weakness of it: `latest` normally carries
# this build's parser version, so it is the one case where leg B reads the
# predecessor's rows instead of reparsing them, which makes its B-vs-C
# comparison the only assertion here that can fail on the NEXT missing
# parser-version bump. The pinned legs assert this build has moved past 3 and
# past 4, and that stays true forever.
#
# THE CANARY. That comparison only carries weight while leg B really serves the
# released rows, and the shard bytes cannot say whether it did: a cache-format
# migration deserializes the released rows, serves them and re-persists them,
# a discard parses the transcripts again and writes the same bytes, and on
# disk the two are one event. The report can say. The fixture copy gets one
# extra transcript, `session-canary`, 40 KiB long, whose single usage row sits
# in a byte range none of the fingerprint's five 4 KiB sample windows covers
# (`sample_offsets` in message_cache.rs). Once A and C have cached it, its
# `inputTokens` is edited in place -- same length, mtime put back -- an edit
# the warm-hit check (`primary_fingerprint_matches`: size, mtime, samples)
# cannot see. Leg B then reports the figure A cached if it served the row,
# whether `Loaded` or `Migrated`, and the edited figure if it discarded the
# shard and read the file again. Leg D, warm on C's own cache after the same
# edit, is the control: a D that reports the edit means this build's
# fingerprint sees it, and the canary decides nothing this run.
set -euo pipefail

THIS="${1:?usage: $0 <this-binary> <released-binary> <fixture> <released-parser-version|unpinned>}"
PREV="${2:?usage: $0 <this-binary> <released-binary> <fixture> <released-parser-version|unpinned>}"
FIXTURE_NAME="${3:?usage: $0 <this-binary> <released-binary> <fixture> <released-parser-version|unpinned>}"
PREV_PARSER_VERSION="${4:?usage: $0 <this-binary> <released-binary> <fixture> <released-parser-version|unpinned>}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURES="${DSH_GATE_FIXTURES:-$ROOT/crates/tokscale-core/tests/fixtures}"
SOURCE_FIXTURE="$FIXTURES/$FIXTURE_NAME"
OUT="${DSH_GATE_OUT:-$(mktemp -d)}"

[[ -x "$THIS" ]] || { echo "this binary is not executable: $THIS" >&2; exit 2; }
[[ -x "$PREV" ]] || { echo "released binary is not executable: $PREV" >&2; exit 2; }
[[ -f "$SOURCE_FIXTURE/expected.json" ]] || { echo "no fixture at $SOURCE_FIXTURE" >&2; exit 2; }
[[ "$PREV_PARSER_VERSION" == "unpinned" || "$PREV_PARSER_VERSION" =~ ^[0-9]+$ ]] || {
  echo "released parser version must be a number or 'unpinned': $PREV_PARSER_VERSION" >&2
  exit 2
}

# Absolute from here on, because the legs run from a directory of their own:
# the workflow passes `target/release/tokscale`, which means nothing outside the
# repo root.
THIS="$(cd "$(dirname "$THIS")" && pwd)/$(basename "$THIS")"
PREV="$(cd "$(dirname "$PREV")" && pwd)/$(basename "$PREV")"
SOURCE_FIXTURE="$(cd "$SOURCE_FIXTURE" && pwd)"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

# A scan derives project-local roots from the working directory -- OmO's
# `.omo/senpi-task/children` and Prime Agent's `.prime/agent/settings.json`, in
# `scanner.rs` -- so the answer moves with wherever the caller happened to
# stand. Run from a working copy of this repo that had an `.omo` directory in
# it, the 621-input fixture report came back carrying four more models and
# 1,290,779 more input tokens. A fresh runner has none of that and never
# noticed; anyone reproducing a red locally would have. Every leg therefore
# runs from an empty directory.
SCRATCH="$OUT/cwd"
mkdir -p "$SCRATCH"

# The legs scan a copy: the canary is planted in it and then edited, and
# neither belongs in the checked-in fixture.
FIXTURE="$OUT/fixture"
rm -rf "$FIXTURE"
cp -R "$SOURCE_FIXTURE" "$FIXTURE"

echo "this:     $("$THIS" --version 2>/dev/null || echo unknown)"
echo "released: $("$PREV" --version 2>/dev/null || echo unknown), DSH parser version $PREV_PARSER_VERSION"
echo "fixture:  $SOURCE_FIXTURE"
echo "          scanned as a copy at $FIXTURE, plus the canary transcript"
echo "out:      $OUT"

cat > "$OUT/canary.py" <<'PY'
"""Plant the canary transcript, and later edit it. THE CANARY in the script header.

    canary.py plant <fixture-copy> <manifest.json>
    canary.py edit  <fixture-copy> <manifest.json>
"""
import json
import os
import pathlib
import sys

# message_cache.rs: FINGERPRINT_SAMPLE_BYTES and FINGERPRINT_SAMPLE_POINTS.
SAMPLE_BYTES = 4096
SAMPLE_POINTS = 5
# Well past the 20 KiB at which five windows stop covering a file, so the gaps
# between them are 5 KiB wide and the usage row has room to sit in one.
TRANSCRIPT_BYTES = 40 * 1024
PROVIDER = "fixture-provider"
MODEL = "fixture-canary-model"
# Same digit count, so the file's size does not move with the edit.
PLANTED, EDITED = 1000, 9000


def sample_windows(size):
    """The byte ranges `sample_offsets` hashes for a file of this size."""
    sample_len = min(size, SAMPLE_BYTES)
    if sample_len == 0:
        return []
    max_offset = max(size - sample_len, 0)
    offsets = (
        [0]
        if max_offset == 0
        else [0, max_offset // 4, max_offset // 2, max_offset * 3 // 4, max_offset]
    )
    return [(offset, offset + sample_len) for offset in sorted(set(offsets))[:SAMPLE_POINTS]]


def line(doc):
    return json.dumps(doc, separators=(",", ":"))


def event(kind, seq, **data):
    return line({"type": kind, "seq": seq, "time": 1780000000000 + seq, "data": data})


def pad(seq, length):
    # An event type the parser does not know is skipped outright (`_ => {}` in
    # dsh.rs), under every DSH parser version this gate pins, so the padding
    # touches no parser state.
    return event("fixture/pad", seq, pad="x" * length)


def usage(seq, input_tokens):
    # One call, one model, no `responseModel` and no compaction: the row every
    # DSH parser version reads the same way, so the only thing that can move
    # its figure is which bytes were read.
    return event(
        "assistant/message",
        seq,
        turn=0,
        step=0,
        message={"id": "msg-canary", "source": {"provider": PROVIDER, "model": MODEL}},
        usage={
            "inputTokens": input_tokens,
            "outputTokens": 100,
            "cacheReadTokens": 10000,
            "cacheWriteTokens": 10,
        },
    )


def plant(fixture, manifest):
    path = fixture / "sessions" / "--fixture--" / "session-canary" / "session.jsonl"
    path.parent.mkdir(parents=True)
    head = "\n".join(
        [
            line({"type": "session", "id": "session-canary", "cwd": "/fixture", "createdAt": 1780000000000}),
            line(
                {
                    "type": "request/header",
                    "seq": 1,
                    "time": 1780000000001,
                    "data": {"header": {"config": {"provider": PROVIDER, "model": MODEL}}},
                }
            ),
            pad(2, 5500),
            usage(3, PLANTED),
        ]
    ).encode() + b"\n"
    tail = pad(4, TRANSCRIPT_BYTES - len(head) - len(pad(4, 0)) - 1).encode() + b"\n"
    body = head + tail
    if len(body) != TRANSCRIPT_BYTES:
        sys.exit(f"the canary came out {len(body)} bytes rather than {TRANSCRIPT_BYTES}")
    path.write_bytes(body)

    needle = f'"inputTokens":{PLANTED}'.encode()
    if body.count(needle) != 1:
        sys.exit(f"the canary carries {body.count(needle)} copies of {needle!r} rather than one")
    start = body.index(needle)
    end = start + len(needle)
    windows = sample_windows(len(body))
    if any(low < end and start < high for low, high in windows):
        sys.exit(
            f"the canary's usage row at bytes {start}..{end} lands inside a fingerprint "
            f"sample window {windows}; an edit there would be seen and the canary decides nothing"
        )
    manifest.write_text(
        json.dumps(
            {"path": str(path), "model": f"dsh/{PROVIDER}/{MODEL}", "planted": PLANTED, "edited": EDITED}
        )
    )
    print(
        f"canary:   {path.relative_to(fixture)}, {len(body)} bytes; inputTokens at bytes "
        f"{start}..{end}, outside the fingerprint's sample windows {windows}"
    )


def edit(fixture, manifest):
    path = pathlib.Path(json.loads(manifest.read_text())["path"])
    before = path.stat()
    body = path.read_bytes()
    old, new = f'"inputTokens":{PLANTED}'.encode(), f'"inputTokens":{EDITED}'.encode()
    if len(old) != len(new) or body.count(old) != 1:
        sys.exit(f"the canary no longer carries exactly one {old!r} of the same length as {new!r}")
    path.write_bytes(body.replace(old, new))
    os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns))
    after = path.stat()
    if (after.st_size, after.st_mtime_ns) != (before.st_size, before.st_mtime_ns):
        sys.exit(
            "could not put the canary's size and mtime back after the edit: "
            f"{before.st_size} bytes at {before.st_mtime_ns} -> {after.st_size} at {after.st_mtime_ns}"
        )
    print(
        f"canary:   inputTokens {PLANTED} -> {EDITED} in place; {after.st_size} bytes and "
        f"mtime_ns {after.st_mtime_ns}, as before"
    )


mode, fixture, manifest = sys.argv[1], pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3])
{"plant": plant, "edit": edit}[mode](fixture, manifest)
PY
python3 "$OUT/canary.py" plant "$FIXTURE" "$OUT/canary.json"

run() { # leg binary home
  local leg="$1" bin="$2" home="$3"
  mkdir -p "$home"
  if ! (cd "$SCRATCH" && env -i HOME="$home" PATH=/usr/bin:/bin:/usr/sbin:/sbin \
      DSH_HOME="$FIXTURE" TZ=UTC "$bin" --json) > "$OUT/$leg.json" 2> "$OUT/$leg.err"; then
    echo "[$leg] the binary exited non-zero:" >&2
    cat "$OUT/$leg.err" >&2
    exit 1
  fi
}

# Copy a leg's DSH shards aside before the next leg can rewrite them.
#
# What leg B reads is the cache leg A left behind, and leg B rewrites it in
# place when it reparses -- so the only moment the predecessor's own rows exist
# on disk is between the two. Nothing in the CLI fails when they were never
# written: `SourceMessageCache::save_if_dirty` treats an unavailable directory,
# an unavailable lock and a failed shard write as warnings and returns, and
# this script hides a successful command's stderr. A leg A that persisted
# nothing therefore leaves B cold-parsing, agreeing with C for the trivial
# reason, and the gate green on a migration it never ran.
#
# Both platforms resolve the shard root under the isolated HOME: `paths.rs`
# hard-codes `$HOME/.config/tokscale` on macOS, and `env -i` leaves Linux with
# no XDG_CONFIG_HOME, so `dirs::config_dir()` lands in the same place.
snapshot_dsh_cache() { # label home
  local label="$1" home="$2"
  local dir="$home/.config/tokscale/cache/source-message-cache-v2/dsh"
  rm -rf "$OUT/cache-$label"
  # An absent directory is a finding, not an error to raise here -- the
  # comparator names the leg that failed to persist and what it expected.
  [[ -d "$dir" ]] && cp -R "$dir" "$OUT/cache-$label"
  return 0
}

# A and C cache the canary as planted; B and D read it after the edit.
rm -rf "$OUT/home-A" "$OUT/home-C"
run A-previous-cold "$PREV" "$OUT/home-A"
snapshot_dsh_cache before-migration "$OUT/home-A"
run C-this-cold "$THIS" "$OUT/home-C"
snapshot_dsh_cache this-cold "$OUT/home-C"
python3 "$OUT/canary.py" edit "$FIXTURE" "$OUT/canary.json"
run B-this-warm "$THIS" "$OUT/home-A"
snapshot_dsh_cache after-migration "$OUT/home-A"
run D-this-warm "$THIS" "$OUT/home-C"

python3 - "$OUT" "$SOURCE_FIXTURE" "$FIXTURE" "$PREV_PARSER_VERSION" <<'PY'
import json
import pathlib
import re
import sys

out = pathlib.Path(sys.argv[1])
source_fixture = pathlib.Path(sys.argv[2])  # expected.json lives here
fixture = pathlib.Path(sys.argv[3])  # the copy the legs scanned
pinned = sys.argv[4]

LEGS = ("A-previous-cold", "B-this-warm", "C-this-cold", "D-this-warm")
LABELS = {
    "A-previous-cold": "A prev cold",
    "B-this-warm": "B this warm",
    "C-this-cold": "C this cold",
    "D-this-warm": "D this warm",
}
TOTALS = ("totalInput", "totalOutput", "totalCacheRead", "totalCacheWrite", "totalMessages")
# Per model, because token totals cannot see attribution. The v3 -> v4 change
# moved DSH usage onto the model the provider reported serving; a fixture whose
# served model differs from the requested one keeps every total identical
# across that change and moves only the split, so a totals-only comparator
# stays green while stale attribution -- and the pricing derived from it --
# survives the migration. Cost itself is left out: the fixture models are
# deliberately unpriced, so cost is zero on both sides of every change here and
# would only add a network dependency to the comparison.
BUCKETS = ("input", "output", "cacheRead", "cacheWrite", "reasoning", "messageCount")
# The canary's share of each total. The totals are sums over the entries
# (`model_report_token_totals` and the `message_count` sum in lib.rs), so
# taking the canary's row back out leaves exactly the fixture's own figure for
# expected.json to be compared with; a build whose totals stopped being that
# sum would fail against expected.json here, which is the right direction.
CANARY_SHARE = {
    "totalInput": "input",
    "totalOutput": "output",
    "totalCacheRead": "cacheRead",
    "totalCacheWrite": "cacheWrite",
    "totalMessages": "messageCount",
}
canary = json.loads((out / "canary.json").read_text())

failures = []


def fail(message):
    failures.append(message)


def report(leg):
    """A leg's answer: the totals it printed, its per-model split, and the canary set apart."""
    doc = json.loads((out / f"{leg}.json").read_text())
    totals = {field: doc.get(field) for field in TOTALS}
    models = {
        f"{entry['client']}/{entry['provider']}/{entry['model']}": {
            bucket: entry.get(bucket) for bucket in BUCKETS
        }
        for entry in doc["entries"]
    }
    canary_row = models.pop(canary["model"], None)
    if canary_row is not None:
        for field, bucket in CANARY_SHARE.items():
            if totals[field] is not None:
                totals[field] -= canary_row[bucket]
    return {"totals": totals, "models": models, "canary": canary_row}


def varint(buf, at):
    """A bincode `DefaultOptions` integer: <251 inline, then 251/252/253 = u16/u32/u64."""
    head = buf[at]
    if head < 251:
        return head, at + 1
    width = {251: 2, 252: 4, 253: 8}[head]
    return int.from_bytes(buf[at + 1 : at + 1 + width], "little"), at + 1 + width


def shards(label):
    """Read the `CachedShardEnvelope` header off every shard in a snapshot.

    The header is the point: it carries the parser namespace and version a
    shard was written under, and `read_shard_with_limit` compares exactly those
    two against the running build before it looks at anything else. The payload
    behind it is only probed for byte substrings -- the transcript path, which
    `CachedPath` stores verbatim on unix, and the `dsh:` dedup keys -- rather
    than decoded, which would mean tracking `CachedSourceEntry`'s layout here.
    """
    directory = out / f"cache-{label}"
    found = []
    for path in sorted(directory.glob("shard-*.bin")) if directory.is_dir() else []:
        buf = path.read_bytes()
        at = 0
        format_version, at = varint(buf, at)
        length, at = varint(buf, at)
        namespace, at = buf[at : at + length].decode("utf-8", "replace"), at + length
        parser_version, at = varint(buf, at)
        length, at = varint(buf, at)
        payload = buf[at : at + length]
        found.append(
            {
                "name": path.name,
                "bytes": buf,
                "format_version": format_version,
                "namespace": namespace,
                "parser_version": parser_version,
                "payload": payload,
                "keys": set(re.findall(rb"dsh:[!-~]{4,}", payload)),
            }
        )
    return found


def contents(found, skip=()):
    """Every shard in a snapshot, by name, as it sits on disk -- minus any holding a skipped transcript."""
    return {
        shard["name"]: shard["bytes"]
        for shard in found
        if not any(path.encode() in shard["payload"] for path in skip)
    }


def parser_versions(found):
    return sorted({shard["parser_version"] for shard in found})


def format_versions(found):
    """The `CACHE_FORMAT_VERSION` a snapshot's shards were written under.

    Checked after the parser identity by `read_shard_with_limit`. A shard
    already in this build's format had no migration to run, so a rewrite of it
    means it was discarded; one a format behind was going to be rewritten
    whatever leg B decided, and only the canary says which.
    """
    return sorted({shard["format_version"] for shard in found})


transcripts = sorted((fixture / "sessions").rglob("session.jsonl"))
before = shards("before-migration")
after = shards("after-migration")
cold = shards("this-cold")


def uncovered(found):
    """The fixture transcripts no shard in a snapshot holds an entry for."""
    return [
        str(path)
        for path in transcripts
        if not any(str(path).encode() in shard["payload"] for shard in found)
    ]


# 1. The predecessor has to have left a usable cache behind, or leg B tested
#    nothing. "Usable" is one entry per fixture transcript, canary included,
#    under the DSH namespace, carrying messages.
if not before:
    # The warnings the CLI emits when it cannot write a cache are the whole
    # explanation, and this script otherwise discards a successful command's
    # stderr -- so hand them over rather than sending the reader hunting.
    stderr = (out / "A-previous-cold.err").read_text().strip() or "(A wrote nothing to stderr)"
    fail(
        "A, the previous release, persisted no DSH cache shards -- leg B cold-parsed "
        f"and this run graded nothing. A's stderr:\n{stderr}"
    )
else:
    foreign = sorted({shard["namespace"] for shard in before} - {"dsh"})
    if foreign:
        fail(f"A wrote shards under {foreign} rather than the dsh namespace")
    keys = set().union(*(shard["keys"] for shard in before))
    missing = uncovered(before)
    if missing:
        fail(f"A cached no entry for {len(missing)} fixture transcript(s): {missing}")
    if len(keys) < len(transcripts):
        fail(
            f"A cached {len(keys)} DSH message(s) across {len(transcripts)} transcript(s); "
            "at least one entry carries no messages"
        )

# 2. The pinned predecessor has to be a real predecessor. A release already at
#    this build's parser version writes a cache this build owns, so leg B would
#    serve it and pass whether or not the change under test bumped anything.
this_version = parser_versions(cold)
if len(this_version) != 1:
    fail(f"C, this build cold, wrote DSH parser version(s) {this_version}; expected exactly one")
if pinned != "unpinned" and before:
    pinned_version = int(pinned)
    written = parser_versions(before)
    if written != [pinned_version]:
        fail(
            f"A wrote DSH parser version(s) {written}, expected {pinned_version} -- "
            "the pinned release is not the one this case was written against"
        )
    if this_version == [pinned_version]:
        fail(
            f"this build is still on DSH parser version {pinned_version}, the pinned "
            "release's own -- its cache is served rather than migrated, so this case "
            "cannot fail on a missing parser-version bump"
        )
    if parser_versions(after) != this_version:
        fail(
            f"after leg B the predecessor's cache holds DSH parser version(s) "
            f"{parser_versions(after)} rather than this build's {this_version} -- "
            "leg B did not rewrite the rows it was supposed to migrate"
        )

print(
    f"\nA wrote {len(before)} DSH shard(s) at parser version {parser_versions(before)}, "
    f"cache format {format_versions(before)} "
    f"for a fixture of {len(transcripts)} transcript(s), canary included; "
    f"this build writes parser version {this_version}"
)

expected = json.loads((source_fixture / "expected.json").read_text())
current = expected["current"]
legs = {leg: report(leg) for leg in LEGS}

width = max(len(field) for field in TOTALS)
header = " ".join(f"{LABELS[leg]:>12}" for leg in LEGS)
print(f"\n{'field':<{width}}  {header} {'expected':>10}   (canary's share taken out)")
for field in TOTALS:
    row = " ".join(f"{legs[leg]['totals'][field]!s:>12}" for leg in LEGS)
    print(f"{field:<{width}}  {row} {current['totals'][field]:>10}")

print(f"\nper model ({'/'.join(BUCKETS)})")
for model in sorted(set().union(*(leg["models"] for leg in legs.values()), current["models"])):
    print(f"  {model}")
    for leg in LEGS:
        buckets = legs[leg]["models"].get(model)
        rendered = " ".join(f"{buckets[b]!s:>7}" for b in BUCKETS) if buckets else "absent"
        print(f"    {LABELS[leg]:<12} {rendered}")
    buckets = current["models"].get(model)
    rendered = " ".join(f"{buckets[b]!s:>7}" for b in BUCKETS) if buckets else "absent"
    print(f"    {'expected':<12} {rendered}")

planted, edited = canary["planted"], canary["edited"]
canary_rows = {leg: legs[leg]["canary"] for leg in LEGS}
print(f"\ncanary ({canary['model']}.input: planted {planted}, edited to {edited} between C and B)")
for leg in LEGS:
    row = canary_rows[leg]
    print(f"    {LABELS[leg]:<12} {row['input'] if row else 'absent'}")


def differences(left, right):
    """Where two answers disagree, named field by field and model by model."""
    named = [field for field in TOTALS if left["totals"][field] != right["totals"][field]]
    for model in sorted(set(left["models"]) | set(right["models"])):
        mine, theirs = left["models"].get(model), right["models"].get(model)
        if mine is None or theirs is None:
            named.append(f"{model} (only one side reports it)")
            continue
        named += [f"{model}.{b}" for b in BUCKETS if mine[b] != theirs[b]]
    return named


# 3. The canary has to be usable before anything leans on it: every leg must
#    have parsed it, A and C must have cached what was planted, and D -- this
#    build warm on its own cache, after the edit -- must still report the
#    planted figure. A D that reports the edit means this build's fingerprint
#    sees an in-place edit that keeps size and mtime, so B reparsing the canary
#    would say nothing about the released rows. That is a notice, not a red: a
#    fingerprint that sees more is not a cache that was thrown away.
canary_usable = False
unparsed = [LABELS[leg] for leg in LEGS if canary_rows[leg] is None]
if unparsed:
    fail(
        f"no {canary['model']} row in the report of {', '.join(unparsed)}: the canary "
        "transcript was not parsed, and without it nothing here can tell served from reparsed"
    )
elif (canary_rows["A-previous-cold"]["input"], canary_rows["C-this-cold"]["input"]) != (planted, planted):
    fail(
        f"the canary was planted with inputTokens {planted}, but A cold read "
        f"{canary_rows['A-previous-cold']['input']} and C cold read "
        f"{canary_rows['C-this-cold']['input']}: a parser that reads it differently "
        "leaves the canary with nothing to compare"
    )
elif canary_rows["D-this-warm"]["input"] != planted:
    print(
        f"\nNOTE  D, this build warm on its own cache, reports the canary's edited figure "
        f"{canary_rows['D-this-warm']['input']} rather than the {planted} it cached: this "
        "build's fingerprint sees an in-place edit that keeps size and mtime, so the canary "
        "cannot tell serving from reparsing this run. The served-vs-reparsed check is "
        "skipped and the untouched-shards check leaves the canary's shard out. If the "
        "fingerprint changed on purpose, move the canary's usage row out of whatever it "
        "now samples."
    )
else:
    canary_usable = True

# 4. This build's own answer, and then every leg that reads a cache against it.
cold_differences = differences(legs["C-this-cold"], current)
if cold_differences:
    fail(f"C, this build cold, differs from expected.json on {', '.join(cold_differences)}")

# Where the canary says the released rows should have gone: served on the one
# leg whose predecessor shares this build's identity, parsed again everywhere
# else, since a rejected identity is exactly what the pinned legs are for.
expect_served = pinned == "unpinned" and bool(before) and parser_versions(before) == this_version

if pinned != "unpinned":
    baseline = expected.get("predecessors", {}).get(pinned)
    if baseline is None:
        print(f"no predecessors[{pinned}] baseline in {source_fixture}/expected.json", file=sys.stderr)
        sys.exit(2)
    baseline_differences = differences(legs["A-previous-cold"], baseline)
    if baseline_differences:
        fail(
            f"A, the pinned release, differs from its recorded baseline on "
            f"{', '.join(baseline_differences)}"
        )
else:
    # Only leg A goes ungraded here -- a release that moves has no recorded
    # baseline to hold it to. B, C and D below are graded exactly as they are on
    # a pinned leg, and B against C is the load-bearing one: `latest` normally
    # carries this build's parser version, so leg B *serves* its rows rather
    # than reparsing them, and that comparison is the only assertion in this
    # gate that can fail on the NEXT missing bump.
    #
    # It can only do that while the rows really are served, and a shard this
    # build owns by identity has three fates rather than two.
    # `read_shard_with_limit` checks the parser identity first and the cache
    # FORMAT version afterwards, so the shard is either served as it sits
    # (`Loaded`), deserialized through a wire migration and served
    # (`Migrated`), or thrown away (`Stale`, plus `Invalid` for an oversized,
    # truncated or undecodable one). Only the third costs this leg its meaning:
    # leg B then cold-parses, agrees with C for the trivial reason, and the leg
    # keeps passing while grading nothing -- the same "graded nothing" shape
    # the pinned legs check for on the write side.
    #
    # The canary is what tells the third fate from the first two, and it is
    # asserted below for every leg. The shard bytes add one thing on top of it
    # when the released shards already carry this build's format: no rewrite
    # was due at all, so a rewritten shard is a discarded one even when the
    # canary's own shard was served -- a partial discard, an oversized or
    # truncated shard among sound ones. One format back, a rewrite was due for
    # every shard whatever leg B decided (`Migrated` and `Stale` both land in
    # `rewrite_shards`), so the bytes say nothing and the leg falls back to
    # grading where the rewrite landed. Format bumps are driven by whichever
    # client changed its stored layout, usually not DSH -- three of them are on
    # record in message_cache.rs and none moved DSH's parser version -- so that
    # is the ordinary shape of such a PR, not an exotic one.
    print(
        "\nA, the last published release, is printed and not graded: a release that moves "
        "has no recorded baseline. B, C and D are graded exactly as on a pinned leg, and "
        "B vs C here is what fails on the next missing parser-version bump."
    )
    if before and parser_versions(before) != this_version:
        print(
            f"\nNOTE  the last published release writes DSH parser version "
            f"{parser_versions(before)} and this build writes {this_version}, so leg B "
            "reparsed rather than served: this leg grades nothing about the identity path "
            "this run. That is what a fresh bump looks like from here, and it needs a "
            "pinned step naming the release before it -- once the bump is published, "
            "nothing else covers it."
        )
    elif before and format_versions(before) != format_versions(cold):
        # A rewrite was due for the format alone, so the bytes have stopped
        # saying which fate the rows met; the canary says that. What the bytes
        # can still check is where the rewrite landed: a migration that serves
        # its rows and then fails to re-persist them leaves the released shards
        # sitting in the old format and migrates them again on every scan
        # forever. Same shape as the pinned branch's `parser_versions(after)`
        # check, one column over.
        stale = []
        if format_versions(after) != format_versions(cold):
            stale.append(
                f"cache format {format_versions(after)} rather than this build's "
                f"{format_versions(cold)}"
            )
        if parser_versions(after) != this_version:
            stale.append(
                f"DSH parser version {parser_versions(after)} rather than this build's "
                f"{this_version}"
            )
        missing = uncovered(after)
        if missing:
            stale.append(f"no entry for {len(missing)} fixture transcript(s): {missing}")
        if stale:
            fail(
                "after leg B the released cache has "
                + "; ".join(stale)
                + " -- leg B read those rows one cache format back and left no usable "
                "cache in this build's format behind, so every scan from here migrates "
                "them again"
            )
        print(
            f"\nNOTE  the last published release writes DSH parser version "
            f"{parser_versions(before)}, this build's own, but cache format "
            f"{format_versions(before)} against this build's {format_versions(cold)}, so "
            "a rewrite of its shards was due for the format alone and the untouched-shards "
            "check is skipped: a wire migration and a discard leave the same bytes. The "
            "canary is what separates them here, and this leg still grades that the "
            "rewrite landed in this build's format and identity and covers every fixture "
            "transcript."
        )
    elif before:
        skip = () if canary_usable else (canary["path"],)
        if contents(before, skip) != contents(after, skip):
            fail(
                "the last published release writes DSH parser version "
                f"{this_version}, this build's own, in cache format "
                f"{format_versions(before)}, this build's own, so leg B had no migration to "
                "run -- and it did not leave its shards alone anyway, which leaves being "
                "discarded and reparsed (an oversized or truncated shard, or entries whose "
                "stored key no longer matches this build's). B therefore agrees with C "
                "trivially and this leg can no longer fail on a missing parser-version bump"
            )

# 5. The canary's verdict on leg B. Served means the row came out of the
#    released cache, `Loaded` or `Migrated`; parsed again means the shard was
#    discarded, whichever fate did it.
if canary_usable and before:
    read = canary_rows["B-this-warm"]["input"]
    if expect_served:
        if read == edited:
            fail(
                "B, this build on the released cache, parsed the canary transcript again "
                f"rather than serving the row the release cached: it reports inputTokens "
                f"{edited}, the figure edited into the file after A cached {planted}, which "
                "only a fresh read of the bytes can see. The released rows were discarded, "
                "so B agrees with C for free and this leg can no longer fail on a missing "
                "parser-version bump. What discards a cache this build owns by identity: a "
                "cache format this build no longer migrates (a CACHE_FORMAT_VERSION bump "
                "with no legacy branch for the released format), a shard that failed to "
                "deserialize (that one also warns on stderr), entries whose stored key or "
                "identity no longer matches this build's, or a fingerprint computed "
                "differently from the release's"
            )
        elif read != planted:
            fail(
                f"B reports canary inputTokens {read}, neither the {planted} the release "
                f"cached nor the {edited} now in the file"
            )
        else:
            print(
                f"\ncanary: B reports the {planted} the release cached while the transcript "
                f"reads {edited} -- the released rows were served, not parsed again"
            )
    else:
        if read == planted:
            fail(
                "B, this build on the released cache, served the canary row the release "
                f"cached: it reports inputTokens {planted} where the file now reads {edited}. "
                f"Rows written under DSH parser version {parser_versions(before)} were "
                "supposed to be rejected by this build and parsed again, and were not"
            )
        elif read != edited:
            fail(
                f"B reports canary inputTokens {read}, neither the {planted} the release "
                f"cached nor the {edited} now in the file"
            )
        else:
            print(
                f"\ncanary: B reports the {edited} now in the file, not the {planted} the "
                "release cached -- the released rows were parsed again, as a rejected "
                "identity requires"
            )

warm_differences = differences(legs["B-this-warm"], legs["C-this-cold"])
if warm_differences:
    fail(
        f"B, this build on the previous release's cache, differs from its own cold scan on "
        f"{', '.join(warm_differences)}: rows the previous release wrote were served, not reparsed"
    )

own_differences = differences(legs["D-this-warm"], legs["C-this-cold"])
if own_differences:
    fail(f"D, a warm scan of this build, moved on {', '.join(own_differences)}")

# 6. What this build said on stderr while it read and wrote the cache.
#    `warn_cache_failure_once` prints one line per context -- "tokscale:
#    warning: source message cache ..." -- for a shard that failed to
#    deserialize (`Invalid`, which is the fate a broken wire migration meets:
#    a legacy struct that no longer matches the released payload errors out
#    rather than reading as `Stale`), and for a directory, lock or shard write
#    it could not use. The CLI exits 0 either way, and this script otherwise
#    discards a successful command's stderr, so the CLI's own diagnosis would
#    sit in $OUT/<leg>.err unread while the leg passed.
for leg in ("B-this-warm", "C-this-cold", "D-this-warm"):
    warnings = [
        line
        for line in (out / f"{leg}.err").read_text().splitlines()
        if "source message cache" in line
    ]
    if warnings:
        fail(
            f"{LABELS[leg]} warned about the cache on stderr, the CLI's own account of a "
            "shard it threw away or could not write:\n"
            + "\n".join(f"    {line}" for line in warnings)
        )

print()
for message in failures:
    print(f"FAIL  {message}")
if failures:
    sys.exit(1)
print(
    "PASS  this build matches expected.json cold, and its first scan over the released "
    "cache lands on the same per-model figure"
)
PY
