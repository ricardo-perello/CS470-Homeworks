#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"

"$ROOT/build.sh" >/dev/null
"$ROOT/simulator/target/release/simulator" "$1" "$2"
