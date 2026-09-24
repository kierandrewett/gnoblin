# Gnoblin issue tracking

Gnoblin uses Beads (`bd`) for the dependency-aware work queue and GitHub Issues
as the durable, visible issue list. Keep each actionable bug or task visible in
GitHub so it can be picked up from another machine or session.

- Before choosing work, run `./scripts/gnoblin-issues sync --pull-only`, then
  `bd ready`. Search with `bd search` before filing a duplicate.
- A pre-commit hook runs bidirectional Beads/GitHub sync. Install it once with
  `uv tool run --from pre-commit==4.6.2 pre-commit install`. If offline, skip
  only this hook explicitly with `SKIP=beads-github-sync git commit ...` and
  sync again before the next commit.
- To file from Beads, run `bd create "..."` and immediately publish the
  returned ID with `./scripts/gnoblin-issues push <id>`.
- To file directly in GitHub, use `gh issue create --repo kierandrewett/gnoblin`
  and import it with `./scripts/gnoblin-issues pull <issue-url-or-number>`.
- After changing a synced issue or closing it with `bd close`, publish that ID
  with `./scripts/gnoblin-issues push <id>`.
- Manual syncs should specify `--pull-only` or `--push-only`. The pre-commit
  hook runs bidirectionally with `--prefer-newer`; if that reports a conflict,
  resolve it deliberately before proceeding.
- Beads' dependency graph is stored separately from GitHub Issues. Use
  `bd dolt pull` before coordinating from another clone and `bd dolt push`
  after changes when sharing that graph through the configured GitHub remote.

See `.beads/README.md` for setup and command details.
