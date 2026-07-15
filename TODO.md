# gnoblin TODO

Active work for the patched-GNOME direction, currently pinned to Mutter 49.5
and GNOME Shell 49.6. Completed implementation history belongs in Git; checked
items stay here only when they still form part of an active delivery sequence.

## Validate on a real system

- [ ] Select Gnoblin in GDM and complete a full login, logout, lock, and unlock
  cycle with bring-your-own chrome running.
- [ ] Install the generated Mutter, GNOME Shell, and `gnoblin-session` RPMs,
  then verify both Gnoblin and stock GNOME sessions plus the documented
  downgrade/removal path.
- [ ] Approve, restore, narrow, and revoke persistent Screen Cast and Remote
  Desktop grants through a real portal session.
- [ ] Exercise the Gnoblin Settings panel interactively, including pending and
  error states when the Shell D-Bus service disappears.

## Product work

- [ ] Add the remaining `polkit` feature toggle without weakening the stock
  session's authentication-agent ownership.
- [ ] Add an explicit extension sideload workflow instead of requiring manual
  copies into `~/.local/share/gnome-shell/extensions/`.
- [ ] Validate Kobel Shell as the reference external chrome in a real Gnoblin
  login session; keep its implementation in the separate `kobel-shell` repo.

## Deferred protocols

- [ ] Implement `ext-session-lock-v1` only with the security and real-hardware
  checks in `src/protocols/session-lock/README.md`.
- [ ] Implement `wlr-output-management-unstable-v1` only with transactional
  apply, rollback, and real-display validation from
  `src/protocols/output-management/README.md`.

## Packaging

- [ ] Implement and test the Debian/Ubuntu package split described in
  `packaging/deb/README.md`.
- [ ] Implement and test the Arch Linux package split described in
  `packaging/arch/README.md`.
