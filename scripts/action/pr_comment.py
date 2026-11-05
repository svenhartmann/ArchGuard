#!/usr/bin/env python3
"""Create or update ONE sticky ArchGuard comment on a pull request.

    pr_comment.py --repo OWNER/NAME --pr NUMBER --body FILE

The previous comment is the first one that carries a hidden HTML marker AND
was written by the token's identity (github-actions[bot] for GITHUB_TOKEN),
so a quoted marker in someone else's comment is never touched. Bodies at or
above GitHub's 65,536-character limit are cut with a visible note. Talks to
GitHub only through `gh api` (authenticated by GH_TOKEN), so it needs no
third-party action. Exit codes: 0 posted, 1 gh/API failure, 2 usage error.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

MARKER = '<!-- archguard-pr-comment -->'
AUTHOR = 'github-actions[bot]'
PER_PAGE = 100
MAX_BODY = 65536
NOTE = '\n\n> **Note:** ArchGuard truncated this comment to fit the GitHub size limit; see the job summary for the full text.'


def gh_api(*args, payload=None):
    result = subprocess.run(['gh', 'api', *args], input=payload, capture_output=True,
                            text=True)
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        raise SystemExit(f'pr_comment: gh api {" ".join(args)} failed ({result.returncode})')
    return json.loads(result.stdout or 'null')


def find_marked(repo, pr, author):
    """First comment by `author` carrying the marker, walking every page."""
    page = 1
    while True:
        comments = gh_api(f'repos/{repo}/issues/{pr}/comments?per_page={PER_PAGE}&page={page}')
        for comment in comments:
            login = (comment.get('user') or {}).get('login')
            if login == author and MARKER in (comment.get('body') or ''):
                return comment['id']
        if len(comments) < PER_PAGE:
            return None
        page += 1


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--repo', required=True)
    parser.add_argument('--pr', required=True, type=int)
    parser.add_argument('--body', required=True, type=Path)
    parser.add_argument('--author', default=AUTHOR,
                        help=f'login whose marked comment is updated (default: {AUTHOR})')
    args = parser.parse_args()
    text = args.body.read_text(encoding='utf-8')
    if not text.strip():
        print(f'pr_comment: {args.body} is empty', file=sys.stderr)
        return 2
    body = text if MARKER in text else f'{MARKER}\n{text}'
    if len(body) >= MAX_BODY:
        body = body[:MAX_BODY - len(NOTE) - 1] + NOTE
    payload = json.dumps({'body': body})
    existing = find_marked(args.repo, args.pr, args.author)
    if existing is None:
        comment = gh_api('-X', 'POST', f'repos/{args.repo}/issues/{args.pr}/comments',
                         '--input', '-', payload=payload)
        print(f'created comment {comment["id"]}')
    else:
        comment = gh_api('-X', 'PATCH', f'repos/{args.repo}/issues/comments/{existing}',
                         '--input', '-', payload=payload)
        print(f'updated comment {comment["id"]}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
