#!/usr/bin/env sh
# Erzeugt LCOV-Arbeitsbereichsabdeckung und importiert sie nach .archguard/coverage.json.
# shortcut: nur Default-Features; --all-features nachziehen, wenn Feature-Gaps auffallen.
set -eu
cd "$(dirname "$0")/.."
# Homebrew-Rust hat kein llvm-tools-Component (nur rustup stellt es bereit).
if ! command -v rustup >/dev/null 2>&1; then
  export LLVM_COV="$(brew --prefix llvm)/bin/llvm-cov"
  export LLVM_PROFDATA="$(brew --prefix llvm)/bin/llvm-profdata"
fi
# Test-/Beispiel-/Benchmark-Code bleibt draußen: hält die LCOV unter der 32-MiB-Grenze
# des Imports und zählt keine Testzeilen in die Produktabdeckung.
cargo llvm-cov --workspace --lcov --output-path target/coverage.lcov \
  --ignore-filename-regex '(^|/)(tests|examples|benches)/'
[ -x target/debug/archguard ] || cargo build -p archguard-cli
./target/debug/archguard coverage . --report target/coverage.lcov
