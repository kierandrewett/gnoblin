# QEMU install test — 2026-09-13

This is an end-to-end clean guest test of the documented Fedora installation
paths for Gnoblin and Bingux. The guest used Fedora 43 Workstation Live
(`Fedora-Workstation-Live-43-1.6.x86_64.iso`, SHA-256
`2a4a16c009244eb5ab2198700eb04103793b62407e8596f30a3e0cc8ac294d77`) and a
new 40 GiB qcow2 disk. QEMU used 6 GiB RAM, four vCPUs, KVM, user networking,
SSH forwarded to localhost:2229, and VNC bound to `127.0.0.1:5909`. The guest
user was `luna` (wheel), created through Fedora's graphical first-login setup.

The VM is left running for inspection (QEMU PID 79215 at report time); its
restart command and monitor socket are in
`/home/kieran/.local/share/gnoblin-qemu-tests/2026-09-13/`. The VM disk and
raw logs are outside the repository. The repository directory contains the
report only. VNC was verified listening on `127.0.0.1:5909`.

## Literal documented path

The Fedora Gnoblin path succeeded:

```sh
sudo dnf -y copr enable kierandrewett/gnoblin
sudo dnf -y install gnoblin-session
```

Installed NEVRAs were `gnoblin-mutter-49.5-2.gnoblin.fc43`,
`gnoblin-shell-49.6-2.gnoblin.fc43`, and
`gnoblin-session-49.6-2.gnoblin.fc43`. This is a published COPR build, not a
build of the current Gnoblin source tree. The guide-test revision was Gnoblin
`7925a241f8ee33d6cf00d11c876874b3a3029eb7` and Bingux
`6107dfaa05dae04d3a5d900c8498750a4b702e94`.

The Bingux source guide was followed from a clean clone at the latter commit.
The first literal `make doctor` failed because Fedora's fresh image did not
have `make`. The prerequisite command's `libpulse-devel` package was
unavailable on Fedora 43, so it was rerun with `--skip-unavailable`; Fedora's
`pulseaudio-libs-devel` provider was installed. `dnf builddep
~/bingux/packaging/rpm/bingux.spec` then succeeded. The initial
doctor also required Quickshell. Fedora 43's enabled repositories supplied
`quickshell-0.3.1-2.fc43`, which made doctor pass:

```
Bingux prerequisites are ready.
```

`docs/desktop-shell.md:8-10` pins the Quickshell 0.2.1 API line; the standalone
and README guidance separately requires a matching Qt/Quickshell runtime. No
0.2.1 package was available to
the Fedora 43 guest, so 0.3.1 was an explicit environment deviation and is a
material compatibility risk. The exact doctor, builddep, and install logs are
`bingux-doctor.log`, `bingux-doctor3.log`, `bingux-builddep.log`, and
`bingux-quickshell-doctor.log` in the VM artifact directory.

`time make install-user` completed successfully in `2m22.760s`, installing
under `/home/luna/.local/share/bingux`, linking `/home/luna/.local/bin`, and
installing user-systemd units. The install output printed the expected Gnoblin
drop-in glob. `bingux.target` was enabled and started; `bingux-statusd.service`
ran successfully.

## Graphical/session proof

GDM displayed the Gnoblin choice alongside GNOME and GNOME Classic. Selecting
Gnoblin and logging in produced a real Wayland session:

```
GNOME_SHELL_SESSION_MODE=gnoblin
XDG_SESSION_DESKTOP=gnoblin
XDG_SESSION_TYPE=wayland
XDG_CURRENT_DESKTOP=GNOME:Gnoblin
/usr/lib/gnoblin/bin/gnoblin-session
/usr/lib/gnoblin/bin/gnome-shell
```

`loginctl` showed graphical session 29 as `Type=wayland`, and `gnoblinctl
ping` returned `pong`; `gnoblinctl version` returned `49.6-gnoblin`. Evidence
is in `gnoblin-login.png`, `integration-session.png`, `graphical-proof.log`,
and `gnoblin-journal.log`.

The first Bingux start attempt was made from the documented standalone
install flow. Because the guide says to load the installed drop-in glob from
Gnoblin's `init.lua`, an isolated corrective retry created the documented
user file:

```lua
local g = require("gnoblin")
g.load("/home/luna/.local/share/bingux/share/gnoblin/conf.d/*.lua")
g.load("~/.config/gnoblin/conf.d/*.lua")
```

After logout and a fresh Gnoblin login, the outcome was unchanged. This
distinguishes missing integration from the runtime failure.

## User-facing result and exact failures

Bingux's visible shell did not render a bar, dock, or panels. `bingux.service`
restarted with status `255/EXCEPTION` and logged:

```
ERROR: Failed to load configuration
ERROR: caused by @shell.qml[41:5]: Type SnapAssist unavailable
ERROR: caused by @SnapAssist.qml[232:5]: Type ShellPopup unavailable
ERROR: caused by @ShellPopup.qml[232:13]: BackgroundEffect can only be used as an attached object.
```

`bingux-searchd.service` restarted with status `1/FAILURE` and logged:

```
[bingux-searchd] application launcher must be absolute
```

The guest's QEMU/VNC software graphics path also logged Mesa/ZINK device
selection warnings. The QML failure occurred after Quickshell loaded the
configuration, but this test did not reproduce it with a 0.2.1 runtime, so the
0.3.1 compatibility mismatch is suspected rather than proven. `gnoblinctl
status` also reported the missing `/run/user/1000/gnoblin/compositor-v1.sock`;
that compositor functional path remains unverified despite `gnoblinctl ping`
succeeding. `binguxctl status` confirmed there was no running instance. The full bounded journal and command output are in
`full-runtime.log` and `graphical-proof.log`.

## Verdict

Fresh Fedora provisioning, graphical first-login setup, Gnoblin COPR install,
Gnoblin session selection, Bingux clean source checkout, dependency discovery,
doctor, native/user install, systemd target wiring, and Gnoblin session
connectivity all passed. The end-user Gnoblin+Bingux desktop did not pass:
the available Fedora 43 Quickshell package is 0.3.1 while the Bingux docs
require 0.2.1. The QML failure is consistent with that suspected API mismatch
but was not isolated against 0.2.1. Search also has an independent
absolute-launcher configuration failure, and the compositor-v1 socket path
remains unverified. No host desktop/session or host project source was changed.

## Artifact index

All artifacts are under
`/home/kieran/.local/share/gnoblin-qemu-tests/2026-09-13/`, including the
installer screenshots (`review.png`, `progress3.png`), setup screenshots
(`setup*.png`), GDM/session screenshots (`gdm.png`, `sessions.png`,
`gnoblin-login.png`, `integration-session.png`), and the command/journal logs
named above.
