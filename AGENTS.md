# Agent Instructions

This project uses **bd** (beads) for issue tracking.

## Session Start

Run `bd prime` to recover project context (completed stages, active work, prior decisions).

## Issue Tracking

Beads is used for cross-session continuity only:

```bash
bd prime                  # Recover context at session start
bd create --title="..." \ # Before starting a non-trivial task
  --description="..." \
  --type=task --priority=2
bd close <id>             # When task is complete
bd sync --flush-only      # At session end — export to JSONL only
```

Skip in-session status updates (`in_progress`, `blocked`, dependency tracking) —
single-agent work doesn't benefit from them.

## Session End

1. File issues for any remaining or follow-up work
2. Run `cargo test` if code changed — all tests must pass
3. Close completed issues with `bd close`
4. Run `bd sync --flush-only`
5. Commit and push only when explicitly asked to do so
