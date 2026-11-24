- Split work into small tasks. A task is small enough when one agent can finish
  it with its own tests in one pass. Do reads and small edits yourself. Use a
  subagent only for a task that shares no files with other running work.
- If you run on Qwen 3.8 Flash Next: run at most two agents that edit or analyze
  at the same time, counting other sessions in this repo. Check for them first:
  run `git worktree list`, read the task list, and use `ListAgents` if you have
  it. If two are already running, wait or tell the user. Other models have no
  fixed agent limit.
- Every file has one owner at a time. Before you edit, run `git status`. If a file
  you need has changes you did not make, do not edit it. Ask the user or the
  owning session. Tasks that touch the same file run one after another.
- Agents that edit in parallel each work in their own git worktree: start a
  session with `claude -w <name>`, or give a subagent worktree isolation. A new
  worktree starts from the last commit and does not contain uncommitted changes.
  If your task depends on them, stop and tell the user. Return your branch name
  when you finish. Do not merge into main unless the user asks.
- Clean up right after a merge; worktrees with commits are never removed
  automatically. Whoever merges a branch checks that the branch is contained in
  its target (`git merge-base --is-ancestor <branch> <target>`) and that the
  worktree has no uncommitted changes. Then they run `git worktree remove
  <path>`, delete the branch and its `worktree-agent-*` branch, and run `git
  worktree prune`. Leave worktrees of running agents and of other sessions alone.
- Track parallel work in `$(git rev-parse --git-common-dir)/agent-tasks.md`, which
  all worktrees share and git never commits. Before you start, add one line:
  task, owner, files. Remove it when you finish. Do not take a file another line
  already lists.
- Message another session only to resolve a conflict over a file, not for status
  updates. Every message costs that session a model round.

## Agent loop

Larger requests run through the global agent loop: skill `agent-loop`, model in
`~/.claude/skills/agent-loop/BLUEPRINT.md`. Specs and tickets live in `.scratch/<effort>/`
(local, excluded via `.git/info/exclude`, see `docs/agents/issue-tracker.md`). A global hook
blocks `git merge` of commits without an Opus review approval.

- Base branch: `main`. Integration branch: `integ/<effort>`; merging it into `main` needs the user.
- Gate commands, run in the ticket worktree, all must exit 0:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --locked -- -D warnings`
  - `cargo test --workspace --locked`
  - Inner loop while working on one reader: `cargo test -p archguard-flow-<family>` (`common`, `clike`,
    `backend`, `jvm`, `dotnet`, `python`, `web`, `node`, `server`, `rust`); the gate above still runs the workspace.
  - `python3 -m unittest discover -s scripts/checks -p 'test_*.py'`
- Review runs the full gate above first; each failure is a finding with category `gate`.
  Build agents run only crate-local checks (fmt, `clippy -p`, `test -p`).
- Review also checks: golden files byte-identical, no weakened or skipped tests,
  no new archguard findings against `main`.
- Reader tickets: the review also runs the spec's `## Abnahme-Probe` command and probes the
  real-code forms listed there (es-indices: qualified constants with equal names, factory
  methods, aliases on non-literal index names, request objects held in variables).
- Readers: every input that is not read yields a gap/diagnostic, never a silent skip
  (non-literal lists, parse errors, excluded paths, unreadable files).
- Heuristics match the name position of the API, not a receiver name or any key with the
  right name; each heuristic has a test with a lookalike that must not match.
- No per-match scan of the rest of the file (string masks, line counts from offset 0,
  `match_indices` per reference): build per-file tables once; lexers keep char, comment and
  text-block state across lines.
- Fail-first check, in the review:
  ```sh
  t=$(git log --format=%H --grep='^test' integ/<effort>..HEAD | tail -1)
  git checkout "$t" -- $(git diff --name-only "$t" HEAD -- 'crates/*/src/*')
  ! cargo test -p <crate> --test <file>        # must be red
  git checkout HEAD -- .                       # restore
  ```
- Build rules (every build brief points here):
  - Files allowed beyond the ticket's `Files:` line, each with a line
    `- [agent rN] Files+ <path>: <reason>` in the ticket: a struct literal broken by a new
    field, one registration call in `services.rs`, a new sibling file in the same directory
    forced by the compactness hook. Anything else: stop with status "blocked".
  - Commit 1 "test: ..." contains only tests/fixtures and fails; put the red assertion line in
    its body. Commit 2+ contains the code.
  - Before writing code, read the ceilings in `scripts/hooks/compact_metrics.py` (method <=60
    lines, cyclomatic <=15, cognitive <=20, nesting <=4, <=6 parameters, <=20 functions per
    module); commit the first small step early so the hook reports before the design is set.
  - Each ticket adds fixtures in its own subdirectory; tests over a shared fixture root assert
    only on their own files, never `report.complete` or `gaps == 0` for the whole root.
  - Cargo in the foreground only, one cargo process per worktree at a time; never
    `run_in_background` for cargo.
  - Performance tests compare two input sizes (ratio, minimum of 3 runs), never an absolute bound.
- The full gate waits for a slot and a calm machine:
  `until mkdir /tmp/archguard-gate.lock 2>/dev/null; do sleep 20; done; trap 'rmdir /tmp/archguard-gate.lock' EXIT;`
  `while [ "$(sysctl -n vm.loadavg | awk '{printf "%d",$2}')" -ge "$(sysctl -n hw.ncpu)" ]; do sleep 30; done`

## Context before code

Use the archguard MCP server before reading or grepping widely; it answers structure from
the graph in bounded pages. The server's own instructions name the tools.

- Before the first edit: `change_context` with the files or symbols you will touch.
- Grep for text the graph cannot answer, not to find callers or dependencies.
- After editing: the gate commands, then `check` and `compare` against your base branch.
