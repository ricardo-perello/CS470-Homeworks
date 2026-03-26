#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/simulator"
cargo build --release
