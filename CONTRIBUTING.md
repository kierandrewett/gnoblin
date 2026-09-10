# Contributing to Gnoblin

Gnoblin is the compositor and session layer. It keeps Mutter window
management, input, login integration and hardware services, then exposes a
small interface for an external shell such as Bingux. Shell widgets belong in
the Bingux repository.

## Development

Initialise the pinned upstream trees and build a private prefix:

```sh
just init
just dev
```

Run the fast checks before sending a change:

```sh
just test
just verify-fast
```

Use `just verify` for the full headless suite. Use `just verify-release` only
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
install test pass. Debian and Arch files are scaffolds until their build and
install checks are complete.

Use small conventional commits. Keep unrelated working-tree changes out of a
commit, and explain the user-visible result in the commit body.
