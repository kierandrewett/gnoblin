# QEMU graphical validation

This runbook validates a fresh Gnoblin build in a disposable Fedora guest
before it is registered on the host. Run artifacts are stored under
`~/.local/state/gnoblin-qemu-e2e/`; the base qcow2 is read-only.

## One command

Start a prepared overlay with 8 GiB RAM and four vCPUs, then run the soak
against the prefix under test:

```sh
run=/path/to/run
GNOBLIN_QEMU_MEMORY_MIB=8192 GNOBLIN_QEMU_VCPUS=4 scripts/qemu-e2e start "$run"
GNOBLIN_QEMU_SOAK_SECONDS=1200 scripts/qemu-e2e-soak "$run" "$PWD/build/validation-install"
```

`qemu-e2e` first uses `virtio-vga-gl` and `egl-headless`. It records that
renderer in `RUN/renderer`; if QEMU cannot start it retries with plain
`virtio-vga`. The latter does not cover GL shader effects.

The soak copies the prefix to the identical absolute guest path, installs the
Gnoblin session entry in the overlay, enables overlay-only GDM autologin, and
captures a QMP screendump after login. It copies user Gnoblin and Bingux
configuration while excluding `.env`, keyring, and browser data. The original
effects configuration is restored from the specified pre-isolation backup.

## Evidence produced

Each `RUN/soak-TIMESTAMP/` directory contains the installed version, autologin
and post-soak screendumps, QMP input transcript, session summary, user journal,
RSS/PSS samples every ten seconds, ping latency, reload outcomes, and a
`summary.txt` with PASS or FAIL. The loop sends QMP Alt+Tab, clicks, Ctrl+Tab,
and media keys, and reloads every three minutes.

PASS requires a graphical Gnoblin process, successful copy and session setup,
reloads, RSS/PSS and ping samples, and no recorded ping above 250 ms. It is
still necessary to inspect the journal for compositor restart count and
media-key launch timestamps, because those are build-specific log formats.

## Failure handling

Preserve the overlay and artifact directory when a guest fails. The guest is
the only target changed by the runner. Do not register a test prefix or restart
the host session. If the base image cannot be found, record that as a baseline
blocker; an orphaned overlay cannot boot without its backing qcow2.

## Build a guest from a Fedora Cloud image

`scripts/qemu-e2e prepare` needs a base qcow2. Build one with
`scripts/qemu-e2e-provision`. It copies a verified Fedora Cloud image, grows
the disk, and uses cloud-init to create the `luna` user with your SSH public
key and to install GNOME and GDM. The script refuses to overwrite an existing
image, and passwordless sudo exists only inside this disposable image.

```sh
d=~/.local/share/gnoblin-qemu-tests/f43
mkdir -p "$d"
curl -L -o "$d/cloud.qcow2" \
  https://download.fedoraproject.org/pub/fedora/linux/releases/43/Cloud/x86_64/images/Fedora-Cloud-Base-Generic-43-1.6.x86_64.qcow2
sha256sum "$d/cloud.qcow2"   # compare with the Fedora CHECKSUM file
scripts/qemu-e2e-provision "$d/cloud.qcow2" "$d/golden.qcow2"
run=$(scripts/qemu-e2e prepare "$d/golden.qcow2")
scripts/qemu-e2e start "$run"
scripts/qemu-e2e sync-prefix "$run" "$PWD/build/validation-install"
```

## Log in to Gnoblin in the guest

GDM ignores a changed session until AccountsService restarts. In the guest:

```sh
P=/path/to/validation-install
sudo install -Dm644 "$P/share/wayland-sessions/gnoblin.desktop" /usr/share/wayland-sessions/gnoblin.desktop
printf '[daemon]\nAutomaticLoginEnable=True\nAutomaticLogin=luna\n' | sudo tee /etc/gdm/custom.conf
printf '[User]\nSession=gnoblin\nXSession=gnoblin\n' | sudo tee /var/lib/AccountsService/users/luna
sudo systemctl stop gdm && sudo systemctl restart accounts-daemon && sudo systemctl start gdm
```

QEMU cannot take a `screendump` of a GL scanout ("no surface"). Install `grim`
in the guest and capture from inside it with `WAYLAND_DISPLAY=wayland-0 grim`.
QMP `input-send-event` still reaches Gnoblin, so media keys, Alt+Tab and
clicks can be driven from the host.

## Check the scheduling-priority recovery

To reproduce the idle start that a service manager can cause, register a
second session whose `Exec=` is
`/usr/bin/chrt -i 0 /usr/bin/ionice -c3 /usr/bin/nice -n 16 $P/bin/gnoblin`.
Log in to it and run `chrt -p` and `ionice -p` on each `gnoblin` process.
Every process must report `SCHED_OTHER` and a best-effort I/O class. The
journal line `gnoblin-guardian: scheduling policy 5->0` records the reset.
This reproduces the inherited state only. It does not run Ananicy itself.

## Check crash reporting

Kill the compositor and log in again. The next session must show the recovery
panel, and a clean logout must not.

```sh
kill -SEGV "$(pgrep -f 'gnoblin --wayland' | head -n 1)"   # in the guest
cat ~/.local/state/gnoblin/crash-last                          # signal=11 ...
sudo systemctl stop gdm && sudo systemctl start gdm            # log in again
cat /run/user/$(id -u)/gnoblin/config-fallback                 # first line: crash
```

After `gnoblinctl logout`, the next login must have no `crash-last` and no
`crash` notice. The guardian discards a stale crash notice at startup.

## Make the guest match a real install

A bare validation prefix leaves the guest without parts that an installed
package provides. Without them, a session starts but behaves differently:

- `dbus-tools` provides `dbus-update-activation-environment`. Without it the
  session cannot publish `XDG_CURRENT_DESKTOP` and the display variables to
  D-Bus-activated services, and the portal frontend picks the wrong backend.
  `scripts/qemu-e2e-provision` installs it.
- The prefix's systemd user units must be installed under
  `/usr/lib/systemd/user`: `gnoblin-session.target`, `gnoblin-idle.service` and
  `xdg-desktop-portal-gnoblin.service`. Without `gnoblin-session.target` the
  guardian cannot start `graphical-session.target`, and the portal backend
  fails with "Dependency failed".
- Install the portal files too: `share/xdg-desktop-portal/gnoblin-portals.conf`
  and `portals/gnoblin.portal` under `/usr/share/xdg-desktop-portal`, and
  `share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service`
  under `/usr/share/dbus-1/services`.

Check the portal backend with `systemctl --user is-active
xdg-desktop-portal-gnoblin` and `coredumpctl list | rg portal-gnoblin`.

## Test more than one monitor

`scripts/qemu-e2e start` gives the guest one display. Set
`GNOBLIN_QEMU_OUTPUTS=2` to add a second virtual monitor:

```sh
GNOBLIN_QEMU_OUTPUTS=2 scripts/qemu-e2e start "$run"
```

QEMU allows only one GL-accelerated virtio GPU, and an extra virtio-gpu head
stays disconnected until a VNC client attaches. The harness therefore adds one
extra non-GL virtio GPU per extra monitor. The guest sees it as `card1-Virtual-2`.
Mutter logs `Zero-copy disabled for /dev/dri/card1` for it, which is expected.

Set the layout with `gdctl`, for example a 1280x800 monitor at scale 1 and a
3840x2160 monitor at scale 2 to its right. Scale 2 needs a mode of at least
1600x960.

## Repeatable regression checks

`tests/qemu-e2e/` holds scripts that check a synced prefix in the guest and print PASS
or FAIL per check: `run-regression.sh` for capabilities, `commands.capture`, polkit and
keyring prompts, input sources, and compositor stability, and `run-shortcut.sh` for dynamic
shortcuts, and `run-unfocused.sh` for focus-driven window rules. Run `run-prompt-setup.sh` once to create the keyring and GPG fixtures. See
`tests/qemu-e2e/README.md`. Each run saves its output in the run directory.
