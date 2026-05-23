#!/usr/bin/env bash
set -euo pipefail

# Align local toolchains with workspace requirements.
rustup toolchain install 1.88 --profile minimal --component clippy,rustfmt
rustup default 1.88
rustup toolchain install nightly --profile minimal --component rustfmt

# Used across CI-like local checks.
if ! command -v cargo-hack >/dev/null 2>&1; then
  cargo install cargo-hack
fi

cargo fetch

# Add clippy alias so `clippy` works as shorthand for `cargo clippy`.
for rc in "${HOME}/.bashrc" "${HOME}/.zshrc"; do
  if [[ -f "$rc" ]] && ! grep -q 'alias clippy' "$rc"; then
    echo "alias clippy='cargo clippy'" >> "$rc"
  fi
done
