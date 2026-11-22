#!/usr/bin/env bash
# Line coverage for the `voussoir` library's own tests, and the floor that gates it.
#
# Uses the LLVM tools the toolchain already links against (`llvm-profdata`,
# `llvm-cov`) instead of `cargo llvm-cov`, so it needs no extra install and the
# number it prints is the same number a developer gets locally.
#
# Why a floor at all: coverage is not a quality score, but a large unexplained
# drop means code was added without a test, which is worth failing a build over.
# The floor sits a point or two below the measured baseline on purpose. A gate set
# at the current number fails on unrelated honest changes and gets deleted; one
# set just under it catches a real regression and nothing else.
#
# Baseline: 67.58% lines measured on 2026-11-21 with the default features
# (`full,rt,storage`), tests and fuzz modules excluded. Re-measure and raise the
# floor when a release closes a real gap; never lower it to make a build pass.
#
# Usage:
#   scripts/coverage.sh              # report, fail under the floor
#   VOUSSOIR_COV_FLOOR=0 scripts/coverage.sh   # report only
set -euo pipefail

FEATURES="${VOUSSOIR_COV_FEATURES:-full,rt,storage}"
FLOOR="${VOUSSOIR_COV_FLOOR:-65}"
WORK="${VOUSSOIR_COV_DIR:-target/coverage}"
CRATE="${VOUSSOIR_COV_CRATE:-voussoir}"

# `rustup component add llvm-tools-preview` installs the binaries inside the
# toolchain instead of on PATH, which is exactly where CI's rust-step puts them.
if ! command -v llvm-profdata >/dev/null || ! command -v llvm-cov >/dev/null; then
  sysroot="$(rustc --print sysroot)"
  host="$(rustc -vV | sed -n 's/^host: //p')"
  if [ -d "$sysroot/lib/rustlib/$host/bin" ]; then
    PATH="$sysroot/lib/rustlib/$host/bin:$PATH"
    export PATH
  fi
fi

command -v llvm-profdata >/dev/null || { echo "coverage: llvm-profdata not found (rustup component add llvm-tools-preview)" >&2; exit 2; }
command -v llvm-cov >/dev/null || { echo "coverage: llvm-cov not found (rustup component add llvm-tools-preview)" >&2; exit 2; }

rm -rf "$WORK"
mkdir -p "$WORK"

echo "coverage: building $CRATE with instrumentation (features: $FEATURES)"
# The profile path has to reach the test binaries at run time, not only the
# compiler, so both variables are exported rather than passed inline.
export RUSTFLAGS="-C instrument-coverage"
export LLVM_PROFILE_FILE="$WORK/%p-%m.profraw"

# Keep the binary list so the report measures the exact binaries that just ran,
# rather than whatever happens to be newest in target/.
BINARIES="$WORK/binaries.txt"
cargo test -p "$CRATE" --features "$FEATURES" --lib --no-run --message-format=json \
  | python3 -c '
import json, sys
out = []
for line in sys.stdin:
    line = line.strip()
    if not line.startswith("{"):
        continue
    record = json.loads(line)
    exe = record.get("executable")
    if exe:
        out.append(exe)
print("\n".join(out))
' > "$BINARIES"

test -s "$BINARIES" || { echo "coverage: no test binaries were built" >&2; exit 2; }

# Run them directly: this is where the .profraw files come from.
while IFS= read -r bin; do
  "$bin" >/dev/null
done < "$BINARIES"

llvm-profdata merge -sparse "$WORK"/*.profraw -o "$WORK/merged.profdata"

# Test files are excluded: measuring them would count assertions as coverage of
# the code under test, which flatters the number without meaning anything.
REPORT="$WORK/report.txt"
# shellcheck disable=SC2046
llvm-cov report \
  $(python3 -c '
import sys
print(" ".join("-object=" + b for b in open(sys.argv[1]).read().split()))
' "$BINARIES") \
  -instr-profile="$WORK/merged.profdata" \
  -ignore-filename-regex='(^|/)(tests|fuzz)\.rs$|/rustc/|\.cargo' \
  -sources voussoir/src \
  > "$REPORT"

# The TOTAL row's line-coverage column is the number this gate is about.
LINE_COV=$(awk '/^TOTAL/ { for (i = 1; i <= NF; i++) if ($i == "TOTAL") print $(i + 9) }' "$REPORT")

if [ -z "$LINE_COV" ]; then
  echo "coverage: could not read the total from the report" >&2
  tail -5 "$REPORT" >&2
  exit 2
fi

echo
echo "coverage: line coverage ${LINE_COV} (floor ${FLOOR}%)"
echo "coverage: full report in $REPORT"

# Compare without floating point: strip the % and compare the integer part.
COV_INT=${LINE_COV%%%*}
COV_INT=${COV_INT%%.*}
if [ "$COV_INT" -lt "$FLOOR" ]; then
  echo "coverage: FAIL, line coverage ${LINE_COV} is under the ${FLOOR}% floor" >&2
  echo "coverage: if the drop is honest, add the missing test; do not lower the floor" >&2
  exit 1
fi
echo "coverage: ok"
