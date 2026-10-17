# Render snapshot generation cache (design gate)

## Status

Proposed on 2026-10-12 — **not yet implemented**. Written as the agreed gate
before touching the hottest correctness surface in the terminal renderer
(Fleet CPU-optimization port, item 6 of the 2026-10-12 evaluation; items
1–5, 7, 8a landed first).

## Context

Every paint frame holds the `Term` lock for a full-viewport
`RenderSnapshot::capture` and then resolves colors cell-by-cell outside the
lock (`element.rs` `TerminalElement::snapshot` + `resolve_app_colors`).
Frames unrelated to grid content — cursor blink flips, sidebar/hover
repaints, spinner ticks — pay this in full. The GPUI element path never
consumes `Term` damage (the only consumer is the legacy shell in
`display/mod.rs`), so there is no cheap "grid changed?" signal today.

## Evidence

- `element.rs` paint: lock → `RenderSnapshot::capture` (full grid,
  `nebula_terminal/src/render.rs`) → `resolve_app_colors` per bg-run and
  per text cell.
- `TermDamageState` (`nebula_terminal/src/term/damage.rs`) has `full`,
  `lines`, `last_cursor` — **no revision counter**; plain printable input
  writes via `write_at_cursor` mark no damage (coverage relies on
  read-time cursor patching in `Term::damage()`), so a generation counter
  keyed on damage marks alone would miss ordinary typing.
- Cursor-only movement, selection, and scroll offset change what the frame
  shows without any text write: some movement handlers call `damage_line`,
  but selection (`term.selection`) and display offset are written directly.

## Decision (proposed)

1. `TermDamageState` gains a monotonically increasing **render generation**
   (`AtomicU64`, readable without the `Term` lock). It is bumped at:
   `write_at_cursor` (every printable write — the non-negotiable point;
   Fleet's regression lesson), every existing `damage_*` entry point,
   `resize`, and alt-screen swap. That covers all content mutations;
   non-content inputs go into the cache key instead.
2. Element-side cache of the **resolved** snapshot (plus derived
   `dashed` link cells), stored on the view behind an `Arc`, keyed by:
   generation, rows, cols, cursor point, selection, scrolled-out /
   viewport origin, alt-screen flag, `hint_config` `Arc` identity, palette
   `Arc` identity, and `suggest.line_buf` equality (`prompt_line` derives
   from it). Keying cursor/selection/scroll by value avoids auditing every
   cursor-mutating parser path for damage marks.
3. **Race guard**: read the generation before and after capture under the
   lock; on mismatch the frame still paints but is not cached (PTY reader
   thread wrote concurrently — caching would freeze a mixed frame).

## Rejected alternatives

- Keying the cache on damage marks only: misses `write_at_cursor` typing
  (see Evidence) — frozen text on plain output.
- Skipping `observe_screen`/watchdog work when the snapshot is cached: that
  short-circuits the per-second idle-sample accumulation and pins agent
  state at Running (Fleet's first-attempt regression; see
  `2026-10-12-macos-emoji-strike-quantization.md`'s sibling evaluation).
- Consuming `Term::damage()` from the element path: couples GPUI rendering
  to the legacy damage contract and inherits its read-time cursor patching.

## Consequences (expected)

Unrelated repaints (blink, hover, sidebar) skip the locked capture and the
color-resolve pass entirely. First-frame and content-change frames pay one
extra generation read; cache stores one resolved snapshot per view.
Multi-pane windows benefit proportionally.

## Validation (required when implementing)

- `nebula_terminal` unit tests: generation bumps on `write_at_cursor`,
  each `damage_*` entry, resize, alt-screen swap; no bump on pure reads.
- Full render regression: cursor blink visibility, smooth + resize scroll,
  selection paint, alt-screen swap (TUI fullscreen), emoji cell widening,
  theme switch re-resolve, OSC link hover/dashed cells, prompt-line
  suggestion anchor after typing.
- Manual: run a Claude Code session and `vim` + resize dance; no stale
  frames, no mixed-frame artifacts during output storms.

## Supersedes

None.

## Revisit when

GPUI exposes incremental damage to element clients, or the snapshot shape
grows another view-derived input (extend the key before shipping).
