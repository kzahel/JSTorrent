#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
exec node "$script_dir/package-legacy-app.mjs" "$@"
