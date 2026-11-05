#!/bin/sh
# ArchGuard GitHub Action: render the PR comment for `review --base`.
#
# The base commit must exist locally. actions/checkout fetches one commit by
# default, so a missing base is fetched from origin once (depth 1).
#
# Environment:
#   ARCHGUARD_BIN           binary (default: archguard on PATH)
#   ARCHGUARD_PATH          project path (default: .)
#   ARCHGUARD_BASE          base commit or ref
#   ARCHGUARD_COMMENT_FILE  Markdown output file
#   GITHUB_OUTPUT           step output: comment-file
#   GITHUB_STEP_SUMMARY     the comment is appended to the job summary
set -u

bin="${ARCHGUARD_BIN:-archguard}"
path="${ARCHGUARD_PATH:-.}"
base="${ARCHGUARD_BASE:?ARCHGUARD_BASE is required}"
comment="${ARCHGUARD_COMMENT_FILE:?ARCHGUARD_COMMENT_FILE is required}"

has_base() { git -C "$path" cat-file -e "$base^{commit}" 2>/dev/null; }

if ! has_base; then
    echo "Fetching base commit $base ..."
    git -C "$path" fetch --quiet --no-tags --depth=1 origin "$base" || true
fi
if ! has_base; then
    echo "::error title=ArchGuard::base commit $base is not available; use actions/checkout with fetch-depth: 0"
    exit 3
fi

mkdir -p "$(dirname "$comment")" || exit 3
"$bin" review "$path" --base "$base" --format pr-comment --out "$comment"
code=$?
if [ "$code" -ne 0 ]; then
    echo "::error title=ArchGuard::archguard review failed with exit code $code"
    exit "$code"
fi
echo "comment-file=$comment" >> "${GITHUB_OUTPUT:-/dev/null}"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    cat "$comment" >> "$GITHUB_STEP_SUMMARY"
fi
