# Remote Git write operations (exec channel, command surface mirrors local)

## Status

Accepted 2026-10-14, superseding decisions 3–4 of
[`2026-10-14-remote-drawer-follows-pane-cwd.md`](2026-10-14-remote-drawer-follows-pane-cwd.md)
(the read-only restriction); the follow and root decisions there stand.

## Context

The remote Git view (see the drawer-follow note) shipped read-only with
every write button hidden. User ruling 2026-10-14: the drawer's Git state
for an SSH host task must support stage/commit/push directly instead of
falling back to typing in the terminal.

## Evidence

- The local write surface lives in `display/side_panel/vcs.rs`
  (`git_stage_path` = `add --`, `git_unstage_path` = `restore --staged
  --`, `git_discard_path` = `restore --`, `vcs_commit_message` =
  `commit -m`, `git_push` = `push`, `git_pull` = `pull --ff-only`,
  `git_stage_all` = `add -A`, WSL paths carrying `--no-optional-locks`).
- `ssh_session::exec_capture` already carries equivalent command lines
  (`git -C '<root>' …` with the same escaping rules as the remote
  integration bootstrap).

## Decision

1. Every write goes through the same exec channel:
   `git -C <root> --no-optional-locks <args>`, each argument (paths and
   the commit message included) single-quoted via
   `ai_hook::remote::quote`, so spaces, quotes and `$` survive the remote
   login shell. `--no-optional-locks` matches local: no index.lock
   contention, no disturbing a remote IDE that is open.
2. The command surface maps one-to-one onto local (the seven commands
   above), zero semantic drift — what users learn in the local panel
   transfers to the remote one. Write budget 30 s (push/pull touch the
   network); status reads keep their 8 s budget.
3. Every operation re-fetches status on completion **regardless of
   outcome**: a failed pull/push can still rewrite the working copy or
   refs, and a stale snapshot misleads more than none. While
   `op_running` is set, the remaining operations and the refresh button
   are disabled (same exclusivity as the local side panel).
4. Inline-button dispatch lives in `vcs_panel/rows.rs`; the `VcsList`
   form flag is named `remote` (after writes returned, "readonly" no
   longer described it) — stage/unstage/discard render in both forms and
   dispatch by `remote` to the `remote_git_*` handlers or `side_panel`;
   the merge editor and SVN entries are local-only and gated off. Discard
   keeps its two-step confirmation (`vcs_discard_confirm`; the row key is
   the root-joined relative path, self-consistent per side).
5. The commit draft shares the local `CommitInput`: local and remote are
   two commit destinations of one input, so switching forms keeps the
   draft; `submit_remote_vcs_commit` mirrors the local
   `submit_vcs_commit` interaction (read draft → send → clear).

## Rejected alternatives

- A confirmation dialog per remote write ("these commands run on the
  remote"): the destination is already persistently visible in the host
  row (the same safety design as `remote_files`); per-operation prompts
  only train users to dismiss them. The genuinely irreversible discard
  already has a two-step confirm.
- Fetching a full status before each operation and diffing: one more
  round trip with no incremental benefit.
- Progress parsing for push/pull beyond completion: the exec channel has
  no incremental output streaming; a spinner plus refresh-on-completion
  matches the local spawn-based behavior.

## Consequences

- Without remote git (minimal containers) writes surface an exec error
  row; the panel does not crash. Remote SVN, the merge-conflict editor
  and line-level diff stats stay unprovided — the remote form is
  Git-only.
- The old note's "read-only" conclusion is superseded here; its follow
  and root decisions are unchanged.

## Validation

- `remote_git::tests::command_line_quotes_root_paths_and_messages`
  (escaping: paths with spaces, an `it's $HOME` message).
- Workspace module suites green (pre-existing machine-local flakes
  verified against a clean tree).
- `python3 scripts/check_architecture.py --base <base>` passes.

## Supersedes

Partial — decisions 3–4 of
[`2026-10-14-remote-drawer-follows-pane-cwd.md`](2026-10-14-remote-drawer-follows-pane-cwd.md).

## Revisit when

- Remote SVN support, merge conflict resolution or line-level diff
  statistics become requirements; or the exec channel gains incremental
  output streaming (progress bars for push/pull).
