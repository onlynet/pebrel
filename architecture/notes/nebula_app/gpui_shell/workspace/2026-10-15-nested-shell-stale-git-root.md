# Invalidating a stale git root inside a nested unintegrated shell

## Status

Accepted 2026-10-15.

## Context

With an SSH host task, entering fish (or any nested interactive shell
without the integration) in the remote terminal and running `cd`: the
file drawer follows normally (fish still emits OSC 7), but the left
project grouping and the right Git panel stay pinned to the project from
before fish was entered (user report 2026-10-15; zsh and other shells
are theoretically the same).

## Evidence

The `_nebula_precmd` hook in `res/shell/bashrc`/`zshrc` emits three
things from one hook: OSC 7 (cwd), the `NEBULA|cwd|branch` title
(cwd/branch) and the `pebrel_git_root` UserVar (the root, **sent only on
the enter/leave-repository change edge**). After entering fish the outer
shell blocks, its precmd goes silent, and of the three channels only
fish's own OSC 7 keeps arriving with every prompt (the 2026-10-10
"stays Running after entering fish" note records the same fact) —
`view.cwd` follows while `view.git_root`/`view.branch` keep their old
values. Both consumers (`task_groups_ssh::child_key`,
`remote_git::remote_git_root`) **trust git_root first**, so the stale
root pins them.

## Decision

1. Consumer-side consistency check with one shared predicate,
   `task_groups_ssh::git_root_covers(root, cwd)`: the root is trusted
   only if it is an absolute path and the cwd is absent (nothing to
   judge staleness against — old behavior) or still under the root
   (equal, or starting with `root/` at a path-component boundary —
   `/srv/work` does not cover `/srv/workflow`); otherwise the root is
   stale — the grouping falls back to the cwd key and the Git panel root
   switches to the cwd (`git -C <cwd> status` resolves the repository
   itself; exit 128 outside renders the "not a repository" empty state).
2. **The view layer never clears git_root**: the change-edge protocol
   reconciles two stored last-values (the shell's `_nebula_git_root` and
   the pane's). Clearing the view side means that exiting fish back into
   the same repository subtree finds the outer side deciding "no change"
   and not re-sending — the correct root would be lost. Staleness is a
   broken *relative* relation between cwd and root, so the verdict
   belongs where that pair is read.
3. The branch chip shares the root's fate (`group_branch`): a stale root
   suppresses the old repository's branch from the cwd group; with an
   empty root (the bare-repository single-line shape) the branch stays
   visible.

## Rejected alternatives

- Clearing `git_root` in the view when an OSC 7 leaves the root: breaks
  the change-edge reconciliation (decision 2).
- Probing the true root via remote `rev-parse --show-toplevel` on
  staleness: contradicts the 2026-10-14 drawer note's "no probing for
  the repository root" stance, and adds one remote round trip per cd in
  exactly the degraded (no-integration/fish) mode.
- Pushing integration scripts into fish: fish's rc syntax is
  incompatible; that is the integration's own iteration, out of scope.

## Consequences

- A cd to the target repository *root* inside fish corrects the
  grouping/panel immediately (cwd is the root); a cd into a
  *subdirectory* lands the group name and panel root on that
  subdirectory (the existing degraded-mode semantics, not a regression).
- Panel write operations anchored at a cwd root align porcelain's
  repository-relative paths only when the cwd is exactly the root (same
  existing boundary of the no-integration form).
- Exiting fish back to bash/zsh restores the reported root at the first
  prompt, with no local state to repair.

## Validation

- `task_groups_ssh::tests::stale_root_no_longer_pins_the_group_once_cwd_leaves_the_repo`
  (left the root / still under it / component boundary / absent cwd).
- `branch_chip_follows_the_root_staleness_verdict` (stale suppression,
  bare repository kept).
- `remote_git::tests::root_prefers_reported_git_root_then_cwd` extended
  (stale root yields to cwd; prefix boundary; absent cwd stays trusted).

## Supersedes

None.

## Revisit when

- The integration starts reporting the root more eagerly than the
  enter/leave edge, or fish gains integration support.
