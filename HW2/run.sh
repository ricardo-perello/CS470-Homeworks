#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"

"$ROOT/build.sh" >/dev/null
"$ROOT/scheduler/target/release/scheduler" "$1" "$2" "$3"
