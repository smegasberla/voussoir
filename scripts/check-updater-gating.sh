#!/usr/bin/env bash
# Is `voussoir/src/updater` gated the same way in both directions?
#
# The module compiles on every target, but only part of it applies on mobile:
# Android and iOS let the store own installation, so `install` and the artifact
# half of `manifest` are gated *as modules*.
#
# The trap is a mismatch only one target sees. A `use` that is not gated importing
# an item from a gated module compiles on Linux and fails on Android — exactly how
# `--features full` stopped building for mobile while every local check stayed
# green. This script catches that shape in milliseconds;
# `scripts/check-mobile-targets.sh` cross-compiles for the truth, and this one runs
# where no NDK is installed.
#
# Usage: scripts/check-updater-gating.sh
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

python3 - <<'PY'
import re, sys

BASE = "voussoir/src/updater/"
GATE = 'target_os = "android"'

def gate_above(lines, i):
    """Whether the attribute block immediately above line `i` carries the gate."""
    j = i - 1
    while j >= 0:
        prev = lines[j].strip()
        if prev.startswith("#["):
            if "cfg" in prev and GATE in prev:
                return True
            j -= 1
            continue
        if prev.startswith("//"):
            j -= 1
            continue
        break
    return False

problems = []

# 1. Which modules are gated, declared in `mod.rs`.
mod_src = open(BASE + "mod.rs", encoding="utf8").read().split("\n")
mod_gate = {}
for i, line in enumerate(mod_src):
    m = re.match(r'^mod (\w+);', line)
    if m:
        mod_gate[m.group(1)] = gate_above(mod_src, i)

if not mod_gate:
    problems.append("voussoir/src/updater/mod.rs: no `mod` declarations found")

# `fetch` must stay ungated: `update.check` needs it on every target, and it has a
# real implementation on each one.
if mod_gate.get("fetch") is True:
    problems.append(
        "voussoir/src/updater/mod.rs: `fetch` is gated, but `update.check` needs it "
        "on every target; it has a per-platform body instead"
    )

# 2. Every import from a gated module must itself be gated, and every import from
#    an ungated one must not be. Either mismatch is a target-specific build break.
for i, line in enumerate(mod_src):
    stripped = line.strip()
    if not stripped.startswith("use ") and not stripped.startswith("pub use "):
        continue
    m = re.match(r'^(?:pub )?use ([\w:]+)::(?:\{([^}]*)\}|(\w+));', stripped)
    if not m:
        continue
    path, body, single = m.group(1), m.group(2), m.group(3)
    module = path.split("::")[-1]
    if module not in mod_gate:
        continue
    names = [n.strip() for n in body.split(",") if n.strip()] if body else [single]
    import_gated = gate_above(mod_src, i)
    target_is_gated = mod_gate[module]
    if target_is_gated and not import_gated:
        problems.append(
            f"voussoir/src/updater/mod.rs:{i + 1}: imports from gated module "
            f"`{module}` without a gate: {', '.join(names)}"
        )
    if import_gated and not target_is_gated:
        problems.append(
            f"voussoir/src/updater/mod.rs:{i + 1}: imports from ungated module "
            f"`{module}` behind a gate: {', '.join(names)}"
        )

# 3. A gated *module* should not repeat the same gate on its own items: with the
#    module gated they are dead, and they are how the files came to look half
#    protected in the first place.
for module, gated in mod_gate.items():
    if not gated:
        continue
    path = BASE + module + ".rs"
    try:
        src = open(path, encoding="utf8").read().split("\n")
    except FileNotFoundError:
        problems.append(f"{path}: declared as a module but does not exist")
        continue
    for i, line in enumerate(src):
        if line.strip().startswith("#[cfg(") and GATE in line:
            problems.append(
                f"{path}:{i + 1}: the module is already gated in `mod.rs`; this "
                f"item gate is dead and hides which gate actually applies"
            )

if problems:
    print("updater gating check FAILED")
    for problem in problems:
        print("  " + problem)
    sys.exit(1)

print("updater gating check ok")
PY
