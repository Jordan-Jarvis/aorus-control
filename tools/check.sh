#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo fmt --all --check
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release --all-features

while IFS= read -r script; do
  bash -n "$script"
done < <(find . -path ./target -prune -o -type f -name '*.sh' -print)

