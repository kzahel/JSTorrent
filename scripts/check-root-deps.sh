#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if node -e '
  const dependencies = require(process.argv[1]).dependencies ?? {};
  process.exit(Object.keys(dependencies).length === 0 ? 0 : 1);
' "$root_dir/package.json"; then
  echo "Success: root package.json contains only shared development tooling."
else
  echo "ERROR: root package.json contains runtime dependencies."
  echo "Runtime dependencies belong to the workspace package that uses them."
  exit 1
fi
