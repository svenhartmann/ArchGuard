#!/bin/sh
# ArchGuard GitHub Action: run `archguard check` as SARIF and gate on its exit code.
#
#   check.sh run    runs the check, writes SARIF, records the exit code
#   check.sh gate   fails the job according to the recorded exit code
#
# `run` defers exit 1 (gate violations) so the PR comment can still be
# posted; `gate` then decides. Exit 2 (usage) and 3 (analysis/IO/config)
# or anything else fail at once and are never swallowed.
#
# Environment:
#   ARCHGUARD_BIN                binary (default: archguard on PATH)
#   ARCHGUARD_PATH               project path (default: .)
#   ARCHGUARD_SARIF              SARIF output file (run)
#   ARCHGUARD_FAIL_ON_VIOLATION  true or false
#   ARCHGUARD_EXIT_CODE          exit code recorded by run (gate)
#   GITHUB_OUTPUT                step outputs: exit-code, sarif-file
set -u

bin="${ARCHGUARD_BIN:-archguard}"
fail_on_violation="${ARCHGUARD_FAIL_ON_VIOLATION:-true}"

case "$fail_on_violation" in
    true | false) ;;
    *) echo "::error title=ArchGuard::fail-on-violation must be true or false, got '$fail_on_violation'"
       exit 2 ;;
esac

run() {
    sarif="${ARCHGUARD_SARIF:?ARCHGUARD_SARIF is required}"
    mkdir -p "$(dirname "$sarif")" || exit 3
    "$bin" check "${ARCHGUARD_PATH:-.}" --format sarif > "$sarif"
    code=$?
    {
        echo "exit-code=$code"
        echo "sarif-file=$sarif"
    } >> "${GITHUB_OUTPUT:-/dev/null}"
    case "$code" in
        0) echo "ArchGuard check passed." ;;
        1) echo "ArchGuard check found gate violations; see $sarif." ;;
        *) echo "::error title=ArchGuard::archguard check failed with exit code $code (2 = usage error, 3 = analysis, IO or config error)"
           exit "$code" ;;
    esac
}

gate() {
    code="${ARCHGUARD_EXIT_CODE:?ARCHGUARD_EXIT_CODE is required}"
    case "$code" in
        0) ;;
        1) if [ "$fail_on_violation" = true ]; then
               echo "::error title=ArchGuard::archguard check found gate violations (exit 1)"
               exit 1
           fi
           echo "::warning title=ArchGuard::archguard check found gate violations (exit 1); fail-on-violation is false" ;;
        *) echo "::error title=ArchGuard::archguard check failed with exit code $code"
           exit "$code" ;;
    esac
}

case "${1:-}" in
    run) run ;;
    gate) gate ;;
    *) echo "usage: check.sh run|gate" >&2; exit 2 ;;
esac
