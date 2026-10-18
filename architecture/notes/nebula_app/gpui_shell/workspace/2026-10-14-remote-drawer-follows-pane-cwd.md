# Remote drawer follows the pane cwd (file browser follow + remote Git view)

## Status

Accepted 2026-10-14. The read-only restriction on the remote Git view was
superseded the same day by
[`2026-10-14-remote-git-write-ops.md`](2026-10-14-remote-git-write-ops.md);
everything else here stands.

## Context

Focusing an SSH pane left none of the drawer's three views following the
terminal's directory: the file browser located itself once at bind time via
`remote_cwd::probe` (an SSH exec process-tree probe) and never again — with
multiple panes sharing pooled connections the probe answers `Ambiguous` and
falls back to `/`, and a `cd` after binding was consumed by nobody. The Git
view had no remote form at all and rendered the local `side_panel` snapshot
(the previous local directory's repository state), which was misleading.
The pane's own `TerminalView::remote_cwd()` (OSC 7 report, pane-exact) was
previously consumed only by tab duplication.

## Evidence

- `remote_files.rs::attach_remote_browser` (pre-change: probe only).
- `workspace.rs::sync_side_panel_to_active` early-returns when
  `remote_browser.active()` (correct — the local tree must not follow remote
  paths), but the remote side had no follower.
- `details_panel.rs`'s `PanelView::Git` branch had no SSH routing.

## Decision

1. File browser start and follow both trust `remote_cwd()`: at bind time a
   report navigates directly (`navigate_remote`); only a missing report
   (no integration) falls back to the probe. The follow decision lives in
   the pure function `should_follow` (the `following` flag + cwd ≠ current
   + cwd ≠ in-flight target), consumed every frame by
   `route_remote_browser` — the existing single-authority entry point.
2. Follow semantics mirror the local tree's custom root: entering a
   directory or going up parks the follow; the toolbar "follow current
   terminal and refresh" button (reusing the `files.tree.follow_refresh`
   message) restores it. `following` persists with the pane snapshot
   (`RemoteDirectorySnapshot`); `requested_path` deduplicates in-flight
   requests.
3. Remote Git view (`remote_git.rs`): the root prefers the reported
   `pebrel_git_root`, falling back to the remote cwd, no probing; data
   comes from one `git -C <root> status --porcelain=v2 --branch -z`
   (root quoted via `ai_hook::remote::quote`), parsed by the shared
   `git_worktree::status::parse_status`, grouped with the same criteria as
   `collect_git_info` (three conflict groups kept). Exit code 128 (not a
   repository / bare) renders the `VcsNotRepository` empty state, not an
   error row.
4. The changes list reuses `VcsList`/`render_vcs_list` with a `remote`
   flag (an added `sync` parameter) gating inline stage/unstage/discard,
   the SVN menu and the merge button; double-click in the remote form
   opens the remote document (`open_remote_document`, root joined as
   strings — never through `PathBuf`, which localizes remote `/` on
   Windows). Plus/minus line stats and history graphs would need extra
   round trips and are not fetched in the remote form.
5. Pane teardown goes through `forget_remote_state` (browser + Git state
   together), shared by both close paths in `workspace.rs`.

## Rejected alternatives

- Keep following via the probe: cannot distinguish multiple panes on one
  host (shared connection) and cannot react to a `cd` (pull-based, keyed
  by destination).
- Read/write remote Git from the start: credentials, conflict and locking
  risk profiles are entirely different; read-only first (user ruling
  2026-10-14, lifted the same day by the write-ops note).
- Forced follow (a `cd` always interrupting manual browsing): inconsistent
  with the local tree semantics (user ruling: match the local tree).

## Consequences

- Without remote integration (no python3 etc.) the file drawer keeps the
  probe → `/` behavior and the Git view shows its empty state — follow
  capability is bound to integration reporting; known boundary.
- Task grouping is unchanged: that chain landed separately (see the
  2026-10-12 SSH grouping notes); non-following cases are mostly stale
  remote rc and self-heal on reconnect.

## Validation

- `remote_files::tests::follow_gate_rejects_disabled_missing_current_and_in_flight`,
  `parked_following_state_returns_with_the_snapshot`.
- `remote_git::tests::root_prefers_reported_git_root_then_cwd`,
  `porcelain_output_maps_to_grouped_snapshot`.
- `python3 scripts/check_architecture.py --base <base>` passes.

## Supersedes

None.

## Revisit when

- The remote Git view grows line-level diff statistics: extend the command
  surface in `remote_git.rs`.
