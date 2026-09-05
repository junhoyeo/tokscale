#!/usr/bin/env bash
# A cache written by the last published release, read once by this build,
# must report what a cold scan of this build reports.
#
#   scripts/check-dsh-cache-migration.sh <previous-binary> <this-binary>
#
# Runs both binaries over crates/tokscale-core/tests/fixtures/dsh-seq-key,
# isolated HOME per leg, DSH_HOME pinned at the fixture, network untouched
# except for the pricing fetch the CLI does on its own:
#
#   A  previous, cold HOME
#   B  this,     warm on A's HOME    the migration leg
#   C  this,     cold HOME
#   D  this,     warm on C's HOME
#
# Passes when C matches expected.json and B and D match C on every field,
# messageCount included. Leg A is printed, never graded: the previous release
# is allowed to be wrong about the fixture, and when it is, B is the leg that
# shows whether a parser identity bump reparsed its rows or served them.
#
# The fixture is the one from tokscale#1187: two unrelated sessions whose
# compaction summaries agree on seq, time and every usage bucket and differ
# only in compactionId (two billed calls), plus a parent and a fork whose
# header lost seedLength (one billed call). A seq-keyed parser drops one of
# the first pair, 3,415 tokens; a compactionId-keyed one keeps both.
set -euo pipefail

PREV="${1:?usage: $0 <previous-binary> <this-binary>}"
THIS="${2:?usage: $0 <previous-binary> <this-binary>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURE="${DSH_GATE_FIXTURE:-$ROOT/crates/tokscale-core/tests/fixtures/dsh-seq-key}"
OUT="${DSH_GATE_OUT:-$(mktemp -d)}"

[[ -x "$PREV" ]] || { echo "previous binary is not executable: $PREV" >&2; exit 2; }
[[ -x "$THIS" ]] || { echo "this binary is not executable: $THIS" >&2; exit 2; }
[[ -f "$FIXTURE/expected.json" ]] || { echo "no fixture at $FIXTURE" >&2; exit 2; }

echo "previous: $("$PREV" --version 2>/dev/null || echo unknown)"
echo "this:     $("$THIS" --version 2>/dev/null || echo unknown)"
echo "fixture:  $FIXTURE"
echo "out:      $OUT"

run() { # leg binary home
  local leg="$1" bin="$2" home="$3"
  mkdir -p "$home"
  if ! env -i HOME="$home" PATH=/usr/bin:/bin:/usr/sbin:/sbin DSH_HOME="$FIXTURE" TZ=UTC \
      "$bin" --json > "$OUT/$leg.json" 2> "$OUT/$leg.err"; then
    echo "[$leg] the binary exited non-zero:" >&2
    cat "$OUT/$leg.err" >&2
    exit 1
  fi
}

rm -rf "$OUT/home-A" "$OUT/home-C"
run A-previous-cold "$PREV" "$OUT/home-A"
run B-this-warm     "$THIS" "$OUT/home-A"
run C-this-cold     "$THIS" "$OUT/home-C"
run D-this-warm     "$THIS" "$OUT/home-C"

python3 - "$OUT" "$FIXTURE/expected.json" <<'PY'
import json, sys
out, expected_path = sys.argv[1], sys.argv[2]
FIELDS = ("totalInput", "totalOutput", "totalCacheRead", "totalCacheWrite", "totalMessages")
exp = json.load(open(expected_path))
correct, seq_keyed = exp["correct"], exp["seq_keyed"]

def totals(leg):
    d = json.load(open(f"{out}/{leg}.json"))
    return {f: d.get(f) for f in FIELDS}

legs = {leg: totals(leg) for leg in ("A-previous-cold", "B-this-warm", "C-this-cold", "D-this-warm")}

w = max(len(f) for f in FIELDS)
print(f"\n{'field':<{w}}  {'A prev cold':>12} {'B this warm':>12} {'C this cold':>12} {'D this warm':>12} {'expected':>10}")
for f in FIELDS:
    print(f"{f:<{w}}  " + " ".join(f"{str(legs[l][f]):>12}" for l in legs) + f" {correct[f]:>10}")

def name(t):
    if t == correct: return "compactionId-keyed"
    if t == seq_keyed: return f"seq-keyed (one summarize call dropped, {exp['dropped_call_total']} tokens)"
    return "neither column"

print(f"\nA, the previous release: {name(legs['A-previous-cold'])}")
failed = False
if legs["C-this-cold"] != correct:
    print(f"FAIL  C, this build cold: {name(legs['C-this-cold'])}; expected the compactionId column")
    failed = True
if legs["B-this-warm"] != legs["C-this-cold"]:
    diff = [f for f in FIELDS if legs["B-this-warm"][f] != legs["C-this-cold"][f]]
    print(f"FAIL  B, this build on the previous release's cache, differs from its own cold scan on {', '.join(diff)}: "
          "rows the previous release wrote were served, not reparsed")
    failed = True
if legs["D-this-warm"] != legs["C-this-cold"]:
    diff = [f for f in FIELDS if legs["D-this-warm"][f] != legs["C-this-cold"][f]]
    print(f"FAIL  D, a warm scan of this build moved on {', '.join(diff)}")
    failed = True
if failed:
    sys.exit(1)
print("PASS  this build lands on the compactionId column cold, and its first scan over the previous release's cache lands on the same figure")
PY
