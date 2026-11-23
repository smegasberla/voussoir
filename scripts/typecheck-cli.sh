#!/bin/sh
# Typecheck the CLI without putting a node_modules tree in the checkout. Bun's
# cache is outside the repository, so a clean tree stays clean and CI has
# nothing to install into it.
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
# Bun refuses to materialise packages under this HOME (it reports success
# and writes nothing), so keep the throwaway tree in the system temp space.
cache="${TMPDIR:-/tmp}/voussoir-typecheck-$(id -u)"

if [ ! -x "$cache/node_modules/.bin/tsc" ]; then
  mkdir -p "$cache"
  (cd "$cache" && bun add typescript@7.0.2 @types/bun@1.4.4 >/dev/null)
fi

exec "$cache/node_modules/.bin/tsc" \
  -p "$root/cli/tsconfig.json" \
  --typeRoots "$cache/node_modules/@types" \
  "$@"
