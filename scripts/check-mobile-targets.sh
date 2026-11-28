#!/usr/bin/env bash
# Cross-check the targets that can be checked on this machine, and say plainly
# which ones cannot.
#
# # Why this exists
#
# `--features full` is the scaffold's default on every platform, and one of the
# features it turns on (`updater`) has a desktop-only half. For a while the item
# gates inside `updater` and the `use` statements above them disagreed, so the
# crate compiled on Linux and failed on Android — and every local check was green
# because the local machine only ever compiles the desktop branch.
#
# A `cfg` is not a claim until something compiles it. This script compiles it.
#
# # What it can and cannot prove
#
# - **Android**: fully checked here. The NDK is a normal Rust target, so a Linux
#   host cross-compiles it and the cfg graph is proved.
# - **iOS**: *not* checked here unless this is a Mac. The target needs the Apple
#   SDK, and without it the build stops in `cc-rs` looking for `xcrun` before any
#   of our code is looked at. That is reported as "unverified", not as "ok" —
#   a skipped check that prints a pass is the failure mode this whole script is
#   a reaction to.
#
# Usage: scripts/check-mobile-targets.sh [--quiet]
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

quiet=0
[ "${1:-}" = "--quiet" ] && quiet=1

# A directory per target, so one target's artifacts cannot mask another's error.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/mobile-check}"
failed=0

say() { [ "$quiet" -eq 0 ] && echo "$@"; return 0; }

check_android() {
  say "checking android (x86_64 + aarch64) ..."
  for target in x86_64-linux-android aarch64-linux-android; do
    if ! rustup target list --installed | grep -qx "$target"; then
      echo "  SKIP $target: not installed (rustup target add $target)" >&2
      continue
    fi
    if out=$(cargo check -p voussoir --target "$target" --features full 2>&1); then
      say "  ok   $target"
    else
      echo "  FAIL $target" >&2
      echo "$out" | grep -E "^error|^ *--> " | head -20 >&2
      failed=1
    fi
  done
}

check_ios() {
  # `xcrun` is the Apple toolchain's shim; without it the cross-compile dies in
  # cc-rs long before our code is parsed, and reporting that as a pass would be a
  # lie about a headline platform.
  if command -v xcrun >/dev/null 2>&1; then
    say "checking ios (aarch64-apple-ios) ..."
    if out=$(cargo check -p voussoir --target aarch64-apple-ios --features full 2>&1); then
      say "  ok   aarch64-apple-ios"
    else
      echo "  FAIL aarch64-apple-ios" >&2
      echo "$out" | grep -E "^error|^ *--> " | head -20 >&2
      failed=1
    fi
  else
    say "  --   ios: unverified (no Apple SDK on this host; the CI job compiles it)"
  fi
}

# The desktop build first: a mobile-only fix that breaks the host is still a break.
say "checking desktop ..."
if out=$(cargo check -p voussoir --features full 2>&1); then
  say "  ok   host"
else
  echo "  FAIL host" >&2
  echo "$out" | grep -E "^error|^ *--> " | head -20 >&2
  failed=1
fi

check_android
check_ios

if [ "$failed" -ne 0 ]; then
  echo "mobile targets: FAILED" >&2
  exit 1
fi
say "mobile targets: ok"
