# Gnoblin Beads workflow

GitHub Issues is the shared, durable list of Gnoblin bugs and tasks. Beads adds
local search, readiness, and dependency tracking. Use the wrapper below so
GitHub authentication and the repository target are set consistently:

```sh
./scripts/gnoblin-issues sync --pull-only
bd ready
```

The wrapper uses `gh auth token` locally, or an existing `GITHUB_TOKEN` in CI.
It never stores a GitHub token in the repository.

## Capture and update work

Install the versioned pre-commit hook once in each clone. Every otherwise-valid
commit then pulls GitHub changes into Beads and pushes local Beads changes back
to GitHub. A sync/auth failure blocks the commit so the issue does not silently
stay local:

```sh
uv tool run --from pre-commit==4.6.2 pre-commit install
```

If offline, `SKIP=beads-github-sync git commit ...` explicitly skips sync for
that commit; sync before committing again.

Create a Bead, then publish it immediately so it appears in the GitHub Issues
tab:

```sh
bd create "Investigate compositor crash during window teardown" -t bug -p 1
./scripts/gnoblin-issues push <bead-id>
```

Import a GitHub issue into Beads:

```sh
./scripts/gnoblin-issues pull <issue-number-or-url>
```

After changing or closing a synced Bead, push that ID again. Use
`./scripts/gnoblin-issues status` to check the GitHub connection. Routine syncs
should specify `--pull-only` or `--push-only`; resolve conflicts deliberately.

The compositor fuzz workflow creates one high-priority Beads bug in GitHub for
real seeded lifecycle-session failures and comments on that issue if later runs
fail again. It does not file compositor bugs for build or patch-application
failures, and it skips issue creation on pull requests.

## Local database and clones

The embedded Dolt database is ignored by Git; `.beads/config.yaml` and
`.beads/metadata.json` hold the shareable project configuration. GitHub Issues
can repopulate the local queue with the pull command above. The configured
Dolt remote can also share Beads dependency history across clones:

```sh
bd dolt pull
bd dolt push
```

On a fresh clone without a shared Dolt data ref, initialize Beads once with a
clean Git index. Beads 1.2.2 automatically commits its initialization files and
also includes anything already staged in Git, so check `git diff --cached`
before running `bd init --prefix gnoblin --skip-hooks --skip-agents`.
