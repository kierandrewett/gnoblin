# Gnoblin roadmap

Gnoblin is pinned to Mutter 49.5 and GNOME Shell 49.6. Keep this file for
unfinished work only; completed investigations and measurements belong in Git
history or the relevant guide.

## Release checks

- [ ] Log in through GDM with a real layer-shell client, then complete logout,
  lock and unlock.
- [ ] Install the generated RPMs on a clean Fedora host. Verify Gnoblin, stock
  GNOME, rollback and removal.
- [ ] Exercise persistent Screen Cast and Remote Desktop grants, including
  narrowing and revocation.
- [ ] Use the Gnoblin Settings panel while the Shell D-Bus service starts,
  stops and returns errors.
- [ ] Re-measure memory, boot time and layer-shell latency on real hardware.

## Product work

- [ ] Replace remaining native authentication, lock and portal interfaces with
  external-shell services. See `docs/native-ui-removal.md` for the boundary.
- [ ] Validate Bingux as the reference external shell in a real login session.
- [ ] Add the remaining `polkit` feature toggle without changing stock GNOME's
  authentication-agent ownership.

## Deferred protocols

- [ ] Implement `ext-session-lock-v1` with the security and hardware checks in
  `src/protocols/session-lock/README.md`.
- [ ] Implement `wlr-output-management-unstable-v1` with transactional apply,
  rollback and real-display validation from
  `src/protocols/output-management/README.md`.

## Performance follow-up

The headless checks already cover reload memory, window churn, boot time and
layer-shell latency. The remaining work is to repeat those measurements in a
logged-in session and tighten the budgets from that data. Do not optimise the
GNOME Shell import list without a new profile: previous measurements found no
useful saving there.

## Packaging

- [ ] Build and test the Debian/Ubuntu package split in `packaging/deb/`.
- [ ] Build and test the Arch package split in `packaging/arch/`.
- [ ] Publish Fedora packages only after the clean-host login and rollback
  checks pass.
