# Sidebar and drawer derive project identity from one pane cwd source

## Status

Accepted 2026-10-15.

## Context

A user reported two breakages plus one architectural ask: ① entering fish
locally made the left grouping follow while the right drawer (file tree +
Git panel) did not; ② entering fish over SSH kept the grouping but the
group header's branch chip vanished (a side effect of that morning's
stale-root verdict: the title protocol goes silent in fish and nothing
backfilled the branch); ③ "can the two sides share one source of cwd
project/directory identity and git state". The user then specified the
target behavior and added: **④** in local fish the branch chip on the
drawer side showed a relic of the old shell (the local chip's only source
was the title protocol); **⑤** in SSH fish the chip showed a path and the
panel wrongly reported "not a repository" (the probe's output-shape
assumption did not hold on the real host).

## Evidence

- The three reporting channels (OSC 7 cwd, the `NEBULA|cwd|branch` title,
  the `pebrel_git_root` UserVar) all originate from the integration
  shell's precmd; unintegrated nested shells like fish leave only OSC 7.
- The left grouping had accepted unix-style cwds and folded
  `/mnt|/cygdrive` since 2026-10-10 (`host_group_path`), but the drawer's
  `TerminalView::local_cwd` did a raw `is_dir` — the mounted form fish
  reports always fails it, so the two sides diverged. The branch chip's
  only source was the title protocol, silent in fish.

## Decision

1. **Local**: the mounted-path fold is single-sourced as
   `shell_detect::mounted_host_path` (`/mnt/<drive>`,
   `/cygdrive/<drive>`, and the Windows-only `/<drive>/` MSYS `$PWD`
   form — on other platforms a single-letter root is a real directory).
   The left grouping (via `host_group_path`) and the right drawer
   (`local_cwd` folds and re-checks when the raw `is_dir` fails) share
   the transform; fish yields the same host directory on both sides.
2. **Remote**: a per-pane project-identity probe
   `refresh_remote_project` hangs off the TitleChanged edge: only when
   the integration-reported root is lost (cwd moved out — that morning's
   staleness verdict) does it run one `git rev-parse --abbrev-ref HEAD
   --show-toplevel` on the remote — the same command, output contract
   and single read-only fork as the integration — storing results in
   `RemoteGit::projects` (cleared on pane close, not on drawer focus
   switches). In-flight probes deduplicate per pane; budget 3 s.
3. **Three consumers read one identity**: the sidebar child-group key
   (`group_root`: reported root → probed root → cwd), the group header
   branch chip (`group_branch`: title while the integration is live,
   probe when stale, bare repositories keep neither), and the Git panel
   root (`remote_git_root`, same precedence). The probed root also fixes
   the cwd fallback anchoring to a repository **subdirectory** —
   porcelain paths are relative to the real root.
4. Zero traffic while the integration is live: a reported root covering
   the cwd short-circuits the probe; the integration stays authoritative
   (precmd is in-band, the probe waits for an event edge).
5. **The local branch also derives from cwd** (④, per the user's spec):
   `project_group_cache` values widen from root strings to
   [`LocalProject`] (root + branch). The upward stat chain for the root
   still runs once per first-seen cwd; the branch re-reads `.git/HEAD`
   per prompt (`git_head_branch`, no git spawn; linked worktrees follow
   the `gitdir:` pointer, detached reports "HEAD" like `--abbrev-ref`).
   The local chip no longer reads the title protocol's `view.branch` —
   in fish it is a relic of the previous repository.
6. **Self-describing probe command** (⑤): two **independent**
   `rev-parse` captures joined by `printf` into one
   `PEBREL_PROJECT\t<branch>\t<root>` line — order and field count
   pinned by the command, the parser only accepting the marker line
   (immune to login-shell banners and git's argument-order variance);
   wrapped in `sh -c` (double quoting) so bash/zsh/fish login shells
   share one syntax; exit code always 0, missing git or non-repository
   just yielding empty fields. **This design died wholesale on the
   user's host** (⑤ retest unchanged) and is superseded by decision 7:
   however clever the single command, it introduced shell-syntax
   assumptions the status channel never had.
7. **The probe returns to two single-purpose execs** (⑤ retest): branch
   and root each run their own `git -C <cwd> rev-parse …` through the
   exact command builder the status channel already uses (quoting and
   argument shapes proven on real hosts), zero new shell syntax; each
   returns a single line or fails (a non-zero exec exit is `Err`, stderr
   never enters results) — no shape assumptions.
8. **Branch-slot defense**: a branch field that looks like an absolute
   path (leading `/`) is never trusted — applied to both the title
   protocol and the probe branch slots (the user's "branch became a
   path" symptom is caught regardless of which upstream slot misfired).
9. **Process-tree fallback for local cwd** (rescuing the "known
   boundary", added by the user on 10-15): nested shells that never
   report OSC 7 (PowerShell→bash/zsh) and cmd panes get their cwd read
   by `platform::pane_cwd` from the **deepest shell process** in the
   pane's process tree via the PEB `CurrentDirectory` (Windows has no
   `/proc`; read-only, same user, `ReadProcessMemory`; the trailing
   separator normalized to the OSC 7 form). Triggering rides the Wakeup
   edge (a cd always produces prompt output) with a 2 s throttle;
   `cwd_reported_at` tracks shell self-reporting liveness and panes that
   reported within the window back off — process enumeration is spent
   only on shells that truly never report. Results enter through
   `adopt_process_cwd`, the exact same entry point as OSC 7 (update
   `cwd` + broadcast TitleChanged), so grouping/drawer/Git panel follow
   together without knowing the origin.
   **First-version incident**: an idle PowerShell pane with no prompt
   output for the window was misread as non-reporting, and the
   reflow output of a maximize/restore happened to trigger the probe edge —
   but PowerShell's `Set-Location` only updates `$PWD`; the Win32 process
   directory stays at the **launch directory**, so both sidebars were
   overwritten back to `C:\Users\…` (reproduced three clicks in a row;
   one Enter producing a prompt corrected it via OSC 7). The fix is
   two-layer:
   a. **Target trust is decided at runtime**: cmd natively follows `cd`;
      cygwin-family shells (bash/sh/dash/zsh/fish) are judged by the
      target process's loaded DLLs — `cygwin1.dll` syncs the Win32
      process directory promptly (trustworthy), `msys-2.0.dll`
      deliberately does not for performance (measured: Git Bash's PEB
      stays at the launch directory after `cd` — trusting it reproduces
      the overwrite incident), WSL's `System32\bash.exe` forwarder has
      no cygwin runtime; PowerShell/pwsh/nu/wsl are always rejected.
      Prefer missing over wrong.
   b. **The self-report backoff window shrinks 4 s → 1 s**: backoff is
      only a cost gate (saving enumeration on reporting panes); misuse
      protection is (a)'s job. A large window cost the probe edge of the
      first cd after a nested shell entered — the user's
      PowerShell→cygwin64 bash case of "needs one more Enter /
      maximize-restore before following".
10. **Button-level fallback for SSH cwd**: the remote "follow current
    terminal" button, when the channel is silent (nested bash/zsh),
    switches to `ssh_sftp::remote_cwd`'s remote process-tree probe (a
    one-shot round trip); the drawer's first open already uses it.
    **Rejected**: periodic automatic probing — one exec per pane every
    few seconds buys nothing the button/first-open paths miss.

## Rejected alternatives

- Probing on every TitleChanged unconditionally: pure waste while the
  integration is live; the change-edge criterion (cwd left the known
  root) mirrors the integration's own UserVar edge protocol.
- Writing probe results back into `view.git_root`: same reasoning as
  that morning's "no clearing/rewriting in the view layer" — the edge
  protocol reconciles two stored last-values (shell and pane), and a
  side-channel write breaks the reconciliation when fish exits back into
  the same repository.
- Teaching fish the local integration: fish is not a POSIX rc; that is
  the integration's own iteration, local or remote.

## Consequences

- A cd into a repository subdirectory inside fish yields the true root
  for both the SSH group name and the panel root (better than the cwd
  fallback); when the probe says "not a repository" the group falls back
  to cwd with no branch chip — identical to the integration's shape.
- Remote exec traffic is bounded: one rev-parse per pane per project
  switch. The local `/<drive>/` fold is Windows-only; WSL guest-native
  paths (`/home`) are not in the host filesystem and keep their WSL
  channel.
- The previous note's "branch goes stale with the root" verdict is
  superseded by the probe backfill here (the verdict itself stays).

## Validation

- `shell_detect::tests::mounted_cwds_fold_to_host_drive_paths`.
- `gitignore::tests::head_file_yields_the_branch_for_grouping`
  (plain/nested branch/detached/worktree pointer/non-repository).
- `task_groups_ssh::tests::probed_identity_takes_over_when_the_reported_root_is_stale`
  and `branch_chip_follows_the_root_staleness_verdict` (including the
  path-shape defense).
- `remote_git::tests::root_prefers_reported_then_probed_then_cwd`
  (three levels + subdirectory true-root anchoring + integration
  authority) and `probe_fields_reject_path_shaped_branches_and_non_posix_roots`.
- `platform::pane_cwd::tests::pane_tree_reports_the_deepest_shells_working_directory`.

## Supersedes

Partial — the "branch stales together with the root" consequence of the
nested-shell stale-git-root verdict (2026-10-15); the verdict itself
stands.

## Revisit when

- The remote integration starts reporting branches alongside the root,
  or the PEB reader needs to trust more shell families (revisit the DLL
  trust table).
