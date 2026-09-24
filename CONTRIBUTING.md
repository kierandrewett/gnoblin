# Contributing to Gnoblin

Gnoblin is the compositor and session layer. It keeps Mutter window
management, input, login integration and hardware services, then exposes a
small interface for an external shell such as Bingux. Shell widgets belong in
the Bingux repository.

## Development

First install the dependencies listed in the
[source installation guide](docs/install-source.md).
Initialise the pinned upstream trees and build a private prefix:

```sh
just setup
just build-source
```

Run the fast checks before sending a change:

```sh
just check
just test-session
```

Use `just test-all` for a fresh build plus the full headless suite. Use `just test-release` only
when the real-host and RPM gates are required. The testing guide records which
checks need a real seat, hardware or a running private session.

## Boundaries

Keep upstream submodules at their pinned commits. Put Gnoblin overlays in
`src/`, protocol changes in `src/protocols/`, and small upstream fixes in
`patches/`. Do not commit build prefixes, generated tarballs or local session
configuration.

## Packaging

Fedora packages use the specs in `packaging/rpm/`. The COPR project is
`kierandrewett/gnoblin`; publish only after the local release gate and a clean
install test pass. Debian and Arch files describe the planned package split
until their build and install checks are complete.

Use small conventional commits. Keep unrelated working-tree changes out of a
commit, and explain the user-visible result in the commit body.

## Lint and format

See [Code quality](docs/code-quality.md) for tool setup, language coverage,
whole-repository checks and formatting selected files.

## Issues and commit hooks

Gnoblin uses Beads for its work queue and GitHub Issues as the shared record.
Install `bd`, authenticate `gh` for the repository, then install the hook once
per clone:

```sh
uv tool run --from pre-commit==4.6.2 pre-commit install
```

The hook syncs Beads and GitHub before each otherwise-valid commit. Commits
require a working GitHub connection and issue-write access. To commit offline,
explicitly skip this hook once with
`SKIP=beads-github-sync git commit ...`, then sync before the next commit. See
`.beads/README.md` for filing and importing issues.
