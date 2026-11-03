- Split work into small tasks. A task is small enough when one agent can finish
  it with its own tests in one pass. Do reads and small edits yourself. Use a
  subagent only for a task that shares no files with other running work.
- Run at most two agents that edit or analyze at the same time, counting other
  sessions in this repo. Check for them first: run `git worktree list`, read the
  task list, and use `ListAgents` if you have it. If two are already running,
  wait or tell the user.
- Every file has one owner at a time. Before you edit, run `git status`. If a file
  you need has changes you did not make, do not edit it. Ask the user or the
  owning session. Tasks that touch the same file run one after another.
- Agents that edit in parallel each work in their own git worktree: start a
  session with `claude -w <name>`, or give a subagent worktree isolation. A new
  worktree starts from the last commit and does not contain uncommitted changes.
  If your task depends on them, stop and tell the user. Return your branch name
  when you finish. Do not merge into main unless the user asks.
- Track parallel work in `$(git rev-parse --git-common-dir)/agent-tasks.md`, which
  all worktrees share and git never commits. Before you start, add one line:
  task, owner, files. Remove it when you finish. Do not take a file another line
  already lists.
- Message another session only to resolve a conflict over a file, not for status
  updates. Every message costs that session a model round.
