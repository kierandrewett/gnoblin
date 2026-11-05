# Graphical VM test

This note describes the graphical VM test: what it checks, how each check fails, which releases it covers, how it
gates a release, and what a green result proves. The code is in `tests/graphical/`, `scripts/run-graphical-test.sh`,
`.github/actions/graphical-test`, the `graphical` job of `.github/workflows/target.yml` (one target leg of the
release) and `.github/workflows/graphical.yml` (a run by hand).

## Purpose

The package tests prove that the Gnoblin packages install. They do not prove that a session starts and draws.
The graphical VM test does that. For each release it boots the distribution's own cloud image in QEMU, installs the
Gnoblin packages of that release, starts a real Gnoblin session on a virtual GPU, and checks the screen and the
input from outside the guest.

The owner's rule: a target publishes its packages only when its build and its graphical test passed, on each
architecture. There is no exemption: an arm64 package needs a passing arm64 guest. A release that fails and is not at
its end of life gets a GitHub issue. A release at its end of life gets no issue, because nobody fixes a release that
its distribution no longer supports. `design/release-pipeline.md` has the gate and the issues.

## Which releases

`scripts/targets.json` holds the policy. `graphical_newest` (5) is the number of the newest releases of each family
that the test covers by hand, by release date, with packages or without. Ubuntu counts long-term support releases
only, as in the package listing. The four rolling targets (openSUSE Tumbleweed, Arch Linux, Debian testing, Alpine
edge) are always in. Every target with packages is in too, inside the window or not, so the release gate has a
graphical result for every target. `graphical_architectures` is `["amd64", "arm64"]`: every architecture of the
targets.

`scripts/supported-targets.py` makes the list from endoflife.date at run time. Its `graphical` output has one
entry for each release, with:

- `name`, `family`, `release`, `slug` and `kind`.
- `eol_date` from endoflife.date, and `eol`: true when that date is today or earlier. A rolling target is never at
  its end of life.
- `packaged`: true when a package job builds the release (state built or experimental). `reason` holds the
  blocker of a release without packages: its `planned_reason`, or "no package job builds it" when the family records
  none.
- `image`: the cloud image data, with the release and the architecture filled in (below). It is null, with the
  reason in `image_missing`, when the system publishes no image for that architecture.
- `guest`: the QEMU guest of the architecture, from `architectures[].guest` in `targets.json` (below).
- `install_artifact` and `install_suffix`: the name of the package artifact of the target.

The `graphical_packaged` output is the entries with packages: the graphical jobs of the release. No workflow and no
script names a release, an image or a family. `python3 scripts/supported-targets.py --table` prints the counts in its
last lines.

### Cloud images

Each family and rolling target in `targets.json` has a `graphical` block. `index` lists the folders that hold the
image; the first one that has the image and its checksum file wins. `file` and `checksum` are regular expressions.
`{release}`, `{codename}` (the endoflife.date codename, lower case, first word), `{arch}` (`amd64`, `arm64`) and the
guest's `{cpu}` (`x86_64`, `aarch64`) and `{boot}` (`bios`, `uefi`) are replaced, and `{file}` in `checksum` is the
image name. A key named after an architecture replaces `index`, `file` or `checksum` there, or holds `none`: the
reason that the system has no image for that architecture. The harness takes the newest name that matches, reads the
checksum file from the same mirror, and checks the download against it. A checksum file of one image may hold the
bare digest (Alpine).

| Family              | amd64 image                              | arm64 image                                | Checksum file                                   |
| ------------------- | ---------------------------------------- | ------------------------------------------ | ----------------------------------------------- |
| Fedora              | Cloud Base Generic `x86_64` qcow2        | Cloud Base Generic `aarch64` qcow2         | `Fedora-Cloud-*-CHECKSUM` (BSD)                 |
| Ubuntu              | `ubuntu-*-server-cloudimg-amd64.img`     | `ubuntu-*-server-cloudimg-arm64.img`       | `SHA256SUMS`                                    |
| Debian              | `debian-*-generic-amd64.qcow2`           | `debian-*-generic-arm64.qcow2`             | `SHA512SUMS`                                    |
| Alpine              | `*alpine-*-x86_64-[bios-]cloudinit-r*`   | `*alpine-*-aarch64-[uefi-]cloudinit-r*`    | `<image>.sha512`                                |
| openSUSE Leap       | `*Leap-*-Minimal-VM.x86_64-*Cloud.qcow2` | `*Leap-*-Minimal-VM.aarch64-*Cloud.qcow2`  | `<image>.sha256`                                |
| openSUSE Tumbleweed | `Minimal-VM.x86_64-Cloud`                | `Minimal-VM.aarch64-Cloud`, ports tree     | `<image>.sha256`                                |
| Arch Linux          | `Arch-Linux-x86_64-cloudimg.qcow2`       | built by `scripts/build-arch-arm-image.sh` | `<image>.SHA256`, arm64: `.sha256` of the build |

Arch Linux publishes no aarch64 cloud image, and Arch Linux ARM ships root file system archives only, with no
cloud-init. So the `arm64` key of Arch Linux has a `build` block, not an index: the harness runs
`scripts/build-arch-arm-image.sh` (as root, on the arm64 runner, natively) and boots the qcow2 that it writes. The
script downloads `ArchLinuxARM-aarch64-latest.tar.gz` from archlinuxarm.org and checks its GPG signature against the
pinned build key `68B3537F39A313B3E574D06777193F152BDBE6A6`. It runs `pacman -Syu` in a chroot, installs cloud-init
(NoCloud only), `cloud-guest-utils` and `openssh`, and writes a GPT with an ESP (systemd-boot on
`\EFI\BOOT\BOOTAA64.EFI`) and an ext4 root. The `image` check passes when the build succeeds and the qcow2 matches the
`.sha256` file that the build wrote. It does not compare against a checksum that someone else published: the trust is
in the signature of the archive and in the signed Arch Linux ARM packages. The image is built fresh in every run, so
it is never older than the run; `image-build.log` and `image-manifest.txt` (tarball checksum, package list) are part of
the evidence. Arch Linux ARM's `os-release` says `ID=archarm`, which `guest.sh` treats as `arch`. The image is built
inside the 40-minute limit of the build block, and the job keeps the downloaded archive in the Actions cache for the
month, so a run does not download it again; the signature is checked again in every run.

What has run and what has not. The signature check is tested without aarch64 (`tests/graphical/arch-arm-image.test.py`:
a changed archive, a changed signature, a signature by another key and another key in the key file each stop the
build, and the shipped key file is the pinned key). The download, signature check, unpack, configure and pack stages
ran on x86_64 under fakeroot on the real archive, and that image booted in an emulated aarch64 QEMU with cloud-init and
SSH working after the packages were added by hand. The `packages` and `finalize` stages (`arch-chroot`, `pacman -Syu`
and `mkinitcpio` in a chroot) and a whole `build-arch-arm-image.sh` run on an aarch64 host have never run, and no
Gnoblin session has run on this guest. The first CI run of the Arch Linux arm64 leg is their first proof.

Fedora moves a release to `archives.fedoraproject.org` at its end of life, so its `index` lists both places.

Debian uses its `generic` image and not `genericcloud`. Both have cloud-init. The `genericcloud` kernel has no DRM
drivers, so it has no virtio GPU, and Gnoblin stops with "No GPUs found". That is a property of the image, not of
Gnoblin.

Debian testing and Alpine edge have no cloud image of their own. Their `graphical` block names a `base` family and an
`upgrade` command. The harness boots the newest release of the base family, runs the command (it points the package
sources at testing or edge and upgrades), and reboots.

## How one run works

`tests/graphical/harness.py` runs on the host. It needs Python 3, QEMU, `qemu-img`, `ssh`, `ssh-keygen`, `tar` and
`genisoimage` (or `mkisofs` or `xorriso`).

1. It downloads and checks the image, and makes a qcow2 overlay of 16 GiB on it.
2. It makes a NoCloud seed. cloud-init creates the user `gtest` and lets root and `gtest` log in with a key that
   the harness makes for the run. There is no password.
3. It starts QEMU with the guest data of the architecture (`architectures[].guest` in `targets.json`): KVM when
   `/dev/kvm` is usable (or TCG), 4 CPUs, 4 GiB, a virtio GPU at 1280x800 with no virgl, a virtio keyboard and a
   virtio tablet, user networking with SSH forwarded to a free local port, and QMP on a socket. amd64:
   `qemu-system-x86_64`, machine `q35` with no PS/2 controller (`i8042=off`), `virtio-vga`, the default BIOS.
   arm64: `qemu-system-aarch64`, machine `virt` (`gic-version=max`), `virtio-gpu-pci`, UEFI firmware as two pflash
   drives (the first code and vars pair of `firmware` that exists; the vars file is copied for the run). `virt` has
   no PS/2 controller, so the virtio devices are the only input there too.
4. It copies `guest.sh` and `wlclient.py` (and the package files) into the guest. `guest.sh install` installs the
   graphics stack (Mesa with llvmpipe, a D-Bus user session and Python 3) and the `gnoblin` package, from the files
   or from repo.gnoblin.org.
5. `guest.sh start` starts the session: a systemd service with `PAMName=login`, `TTYPath=/dev/tty1` and
   `XDG_SESSION_TYPE=wayland` runs the `Exec` line of `/usr/share/wayland-sessions/gnoblin.desktop`. This is how a
   display manager starts a session: a logind session of type wayland on seat0, with a user systemd instance. The
   command runs through `/bin/sh -c exec`, as `gdm-wayland-session` runs it, because SELinux does not let the user
   domain enter a file under `/usr/lib`. Alpine has no systemd: init starts util-linux `login -p -f` on tty1 with
   `XDG_SESSION_TYPE=wayland`. Its PAM stack has `pam_elogind`, which opens the logind session (busybox login has no
   PAM, and Gnoblin refuses to start without `XDG_SESSION_ID`). eudev replaces mdev, because mutter finds the GPU in
   the udev database. The login shell starts a D-Bus session bus at `$XDG_RUNTIME_DIR/bus` and then the session.
6. The checks run (below). Then the harness collects the logs and stops the guest.

The guest renders with Mesa's llvmpipe through the virtio GPU's KMS driver. QEMU reads the scanout for `screendump`,
so the screenshot comes from outside the guest. A virgl GPU would hide the scanout from `screendump`.

### The test client

`tests/graphical/wlclient.py` is the client. It uses the Python standard library only and speaks the Wayland wire
protocol on the compositor socket. Every system that Gnoblin installs on has Python 3, and the `gnoblin` package
depends on it on most of them, so the client needs no package, no toolkit and no compiler in the guest. It is the
same code on every release. A toolkit client such as `foot` differs from release to release, and it cannot report
the input events it gets.

It writes the registry globals to a file, maps a 480x360 `xdg_toplevel` filled with magenta, logs each pointer and
keyboard event of its surface as JSON, and fills the window with green when the key G arrives.

## The checks and how each one fails

The checks run in this order. A check that needs an earlier one is skipped when that one fails, so the first failing
check is the cause. The logs check runs whenever the packages installed.

| Check        | Passes when                                                                                          |
| ------------ | ---------------------------------------------------------------------------------------------------- |
| `packaged`   | The release has packages, and the package source exists.                                             |
| `image`      | The image downloads and its checksum matches the distribution's checksum file.                       |
| `boot`       | The guest boots, cloud-init finishes, and root logs in over SSH.                                     |
| `install`    | The graphics stack and the Gnoblin packages install.                                                 |
| `session`    | The compositor runs, holds a DRM device, `wayland-0` exists, and the session state is `running`.     |
| `globals`    | The client sees every global in `tests/graphical/expected-globals.txt`.                              |
| `window`     | The client maps its window, and `gnoblinctl window list` lists its app ID.                           |
| `screenshot` | A QMP `screendump` is not blank and shows a magenta box of the client's size.                        |
| `pointer`    | QMP pointer events enter the window, move it by the right distance, and click once.                  |
| `keyboard`   | QMP key events reach the client in order, and a second `screendump` shows the green repaint.         |
| `shutdown`   | `gnoblinctl logout` ends the session with status 0, no Gnoblin process is left, nothing dumped core. |
| `logs`       | No `CRITICAL` or `ERROR` line in the session log or the user's journal, outside the allow-list.      |

How each check fails, and what the result names:

- `packaged` fails when the release has no package job. The result is `not packaged` with the blocker from
  `planned_reason`. It counts as a failing release. It also fails when the package folder does not exist.
- `image` fails when no index lists a matching image with a checksum file, when a download fails, or when the
  checksum differs. The detail names the URL and both checksums.
- `boot` fails when QEMU exits, or when SSH does not let root in before the boot timeout (600 s with KVM, 1800 s with
  TCG). The excerpt is the end of the serial log. An `upgrade` command that fails (Debian testing, Alpine edge) fails
  this check too.
- `install` fails when the package manager fails, or when the folder does not hold exactly one `gnoblin` package
  for the release. `install.log` holds the output.
- `session` fails when the session service ends before it is ready (the detail has its exit status), or when within
  120 s (600 s with TCG) there is no compositor process, the compositor holds no `/dev/dri/card*` file (it did not
  take the native KMS backend), there is no `wayland-0` socket, or `gnoblinctl lua ready.lua` does not report the
  runtime and session states `running` (the detail has gnoblinctl's own error). It fails at once when Gnoblin logs
  that its Lua runtime is lost for the session (the supervisor could not recover it, for example "could not suspend
  Mutter for supervisor recovery"), because the runtime cannot come back. The excerpt is Gnoblin's lines from the
  test user's journal, then the session log. A failed `session` also keeps `session-screen.png` and
  `compositor-threads.txt`: each compositor thread's state, CPU time twice 2 s apart, wait channel and kernel stack,
  and an `eu-stack` backtrace (elfutils is installed in the guest for it then, and only then). That tells a busy
  compositor from one that waits, and on what. In run 41905248345 (openSUSE Tumbleweed aarch64, TCG) Mutter logged
  nothing after native control started, the Lua worker gave up waiting for its greeting after 10 s, and the
  supervisor could not suspend Mutter; the session never came up, and the harness waited the full 600 s. The arm64
  sessions of runs 41896060198 and 41905248345 reached `running` in 15 to 31 s, so 600 s is not too short.
- `globals` fails when the client cannot connect or writes no globals, or when a listed global is missing.
- `window` fails when the client does not report `mapped` within 30 s (a protocol error, a missing configure or no
  frame callback), or when `gnoblinctl window list --json` does not list `org.gnoblin.GraphicalTest`.
- `screenshot` fails when the frame has fewer than two colours that each cover 1 % of it, or one colour covers more
  than 99.5 %, or no pixel is near magenta, or the magenta box is not within 10 % of 480x360, or magenta fills less
  than 90 % of its box. The screenshot is saved as PNG either way.
- `pointer` fails when the client sees no pointer enter on its surface, when the last position is more than 3 pixels
  from where the harness put the pointer (it moves 40 and 30 pixels after the first position), or when it does not
  see one left press and one release.
- `keyboard` fails when the key codes that the client sees do not end with G, N, O, B (34, 49, 24, 48), or when the
  second screenshot does not show the green box.
- `shutdown` fails when the session does not end within 60 s of `gnoblinctl logout` (the detail has the exit status
  and message of `gnoblinctl logout` when it failed), when it ends with another status or a signal, when a Gnoblin
  process is left, or when the journal or `coredumpctl` reports a core dump.
- `logs` fails on a line with the word `CRITICAL` or `ERROR` that no rule of the allow-list matches. The allow-list is
  `tests/packages/log-allowlist.txt`, which the package tests own. Until that file exists the harness reads
  `tests/graphical/log-allowlist.txt` if it is there. The two lists are to be one.

When a check that needs the compositor fails (`globals` to `shutdown`), the detail ends with whether the compositor
still answers `gnoblinctl status` within 10 s, and the first such failure keeps `<check>-screen.png` and
`compositor-threads.txt`, as a failed `session` does. In run 41918451629 (Alpine 3.24 amd64, KVM) the window mapped and
the screenshot passed, then `pointer`, `keyboard` and `shutdown` failed: the client saw no input event, and
`gnoblinctl logout` returned after about 5 s (its own reply timeout) without ending the session. Nothing in the logs
said why. The same leg passed in runs 41835867674 to 41905248345, and the input devices were enumerated as in those
runs (the client saw a seat with pointer and keyboard). With this evidence, the next such failure says whether the
compositor stopped answering and where its threads are.

### Proof that each check can fail

`harness.py --break CHECK` makes the guest fail one check on purpose. It is a test of the test.

| `--break`    | What the harness does                                                    |
| ------------ | ------------------------------------------------------------------------ |
| `image`      | It expects a checksum of zeros.                                          |
| `boot`       | The seed holds another key, so SSH never lets the harness in.            |
| `install`    | It copies no package files into the guest.                               |
| `session`    | It unbinds the virtio GPU from its driver before the session starts.     |
| `globals`    | It expects one more global that no compositor has.                       |
| `window`     | It starts the client with a size of 0x0, so the client makes no buffer.  |
| `screenshot` | It stops the client after the window check, so the screen has no client. |
| `pointer`    | It unbinds the virtio tablet from its driver in the guest.               |
| `keyboard`   | It unbinds the virtio keyboard from its driver in the guest.             |
| `shutdown`   | It sends SIGSEGV to the compositor before the logout.                    |
| `logs`       | It writes a `gnoblin-CRITICAL` line to the user's journal with `logger`. |

`globals` is the only check whose break changes the expectation and not the guest: no change to a guest removes a
core global and keeps the session up.

## Evidence

Every run writes a folder:

- `result.json`: the target and its architecture, each check with its status, detail, log excerpt and time, the
  first failure, the guest versions (system, kernel, `gnoblin`, Mesa, Python), the QEMU
  version, the accelerator, the image URL and checksum, and the total time.
- `harness.txt` and `command.txt`: the harness command line and the full QEMU command line.
- `qemu-version.txt`, `qemu.log`, `serial.log`.
- `install.log`, `session.log`, `journal.txt` (the test user's journal), `journal-system.txt`; after the first
  failed check from `session` to `shutdown`, `<check>-screen.png` and `compositor-threads.txt`.
- `screenshot.png` and `screenshot-typed.png`.
- `globals.json`, `client-events.jsonl`, `client.log`, `windows.json`, `gnoblinctl-status.json`, `log-errors.txt`.

The guest disk and the work folder are deleted after the run. The cloud image is deleted too, unless `--keep-image`
is given. The harness prints each check with its time and the time since the start, so a slow first boot on TCG
shows in the log.

## Run one release locally

```sh
scripts/run-graphical-test.sh "Debian 13" /path/to/packages
```

The first argument is a name or a slug from the `graphical` output (`deb13`, `fc42`, `Arch Linux`). The second is a
folder of package files, or `nightly` or `stable` for that channel of repo.gnoblin.org (default `nightly`). The
evidence goes to `build/graphical/<release>/`, or to `$GNOBLIN_GRAPHICAL_OUT`. Other options go to the harness:
`--arch arm64` (the entry of that architecture; the default is amd64), `--accel tcg`, `--break CHECK`,
`--keep-image`, `--min-free-gb N`, `--hold SECONDS` (keep the guest up to look at it), `--deadline SECONDS`.

`--deadline` bounds the whole run. At the deadline the running check fails with "the run reached its deadline of N
s", the later checks are skipped, and `result.json` is written. CI uses it so that a slow guest cannot pass the job
timeout without a result.

## In the release

Every target leg of the release (`.github/workflows/target.yml`, called by `release.yml` once for each target and
architecture) has a `graphical` job after its package suite. It runs when the suite passed, on the runner of the
target's architecture. After a failed suite it is skipped, and the gate reports "not run: suite failed":

1. It downloads the package artifact of the target.
2. `.github/actions/graphical-test` makes `/dev/kvm` usable when the runner has it, installs the runner packages of
   the guest (`host_packages`), runs the harness on the target's graphical entry, and uploads the evidence as
   `graphical-<tag>-<slug>-<arch>`.
3. The job fails when the test fails. An experimental target's job has `continue-on-error`.

The verdict job of the leg copies `result.json` into the leg's verdict, and the release gate reads it: a target whose
graphical test did not pass on its architecture publishes nothing there. A target whose leg has packages and no
`result.json` fails its graphical stage ("did not finish"). `design/release-pipeline.md` describes the gate and the
issues.

## The workflow by hand

`.github/workflows/graphical.yml` runs the test outside a release, by `workflow_dispatch` only. It has three jobs.

1. `targets` reads the `graphical` output of `scripts/supported-targets.py`, filters it by the `releases` and
   `architectures` inputs, and finds the packages: `packages_from` is `nightly-build` (the newest Nightly run that
   built), `artifacts` with `artifacts_run_id`, `nightly` or `stable`.
2. `test` runs one job for each entry, on the runner of its architecture, with the same action as the release. A
   release without packages fails as "not packaged". The jobs have `continue-on-error`: the report decides.
3. `report` runs `tests/pipeline/issues.py --results`. It raises and updates the same issues as the release, and
   only with `dry_run: false` (the default is true). It closes none: it does not see the build stages, so a pass
   here does not show that a release passes.

### KVM and TCG

The amd64 hosted runners have `/dev/kvm` (GitHub enabled it for the Linux runners in April 2024:
<https://github.blog/changelog/2024-04-02-github-actions-hardware-accelerated-android-virtualization-now-available>).
A udev rule makes it usable by the runner user. The job checks `/dev/kvm` and falls back to TCG with a warning.

The arm64 runners have no `/dev/kvm`: GitHub closed the request for nested virtualisation on them as not planned
(<https://github.com/actions/partner-runner-images/issues/147>). So the arm64 guest runs on TCG. TCG is about ten
times slower than KVM, so one release can take hours. The guest data sets the job timeout: `timeout_minutes` is 150
for amd64 and 345 for arm64, under the 360-minute limit of a hosted job. The harness deadline is the job timeout
less 20 minutes, so the evidence is uploaded. With TCG the harness waits longer by default: 1800 s for the boot,
7200 s for an install, 600 s for the session to start and 300 s for the test client. An arm64 target whose guest does
not finish in time fails its graphical stage, and its packages are not published.

Two more settings apply to a guest without KVM:

- systemd waits a short time for each device of `/etc/fstab` (45 s on Fedora 42), and an emulated guest can need
  longer (Fedora 43 and 42 aarch64 went to emergency mode). The harness passes systemd credentials in SMBIOS type 11
  that give every device unit a 10-minute wait (`systemd.unit-dropin.dev-.device`, systemd 256 and newer). On Fedora
  41 and 42, SELinux does not let `systemd-debug-generator` read a credential in the real root, so that drop-in
  applied in the initrd only. A second credential (`systemd.unit-dropin.initrd-switch-root.service`) copies it to
  `/run/systemd/system` in the initrd, before SELinux is loaded, and the real root reads it from there. The cloud
  image is not changed.
- A guest that has given up booting fails `boot` at once, not after the boot timeout. The serial console shows it:
  emergency or rescue mode, sulogin ("the root account is locked"), the audit record of the emergency service, a
  frozen PID 1 or a kernel panic. On Fedora 41 and 42 aarch64 `/dev/console` is tty1 (their `console=ttyS0` does not
  exist on that machine), so systemd's own messages are only on the screen: the harness keeps a picture of it,
  `boot-screen.png`, after a failed boot. An emulated guest then boots once more from a fresh disk, with the reason
  printed and kept in `result.json` (`boot_retry`). The first boot races the host's speed (Fedora 44 aarch64 froze
  after a soft lockup in one run and passed in the runs before), so a second boot is a new draw. A KVM guest is not
  booted again.

The aarch64 guests run on the CPU model `max,pauth-impdef=on`, which is much faster under TCG than `max`. Every
arm64 graphical target has run in CI.

## Issues

A graphical failure is one stage of the release pipeline. `tests/pipeline/issues.py` keeps one issue for each
release, titled `Release pipeline: <name>` with the label `release-pipeline`, for a failure of any stage on any
architecture. `design/release-pipeline.md` has the rules. A "not packaged" result of a run by hand and a planned row
of the release gate are the same failure ("build", not built, with the blocker), so they give one issue, not two.

A run by hand changes issues only with `dry_run: false`. It creates and updates them, and closes none.

## What a green result proves, and what it does not

A green result proves that, on that release and with its packages:

- The Gnoblin packages install on the distribution's own cloud image with its own libraries.
- A session starts the way a display manager starts it, takes the native KMS backend on a virtio GPU, renders with
  software GL (llvmpipe) and reaches the state `running`.
- A Wayland client connects, sees the core globals, maps a window, and the window is on the screen that QEMU reads.
- Pointer and keyboard input from virtual devices reaches the client through libinput, and the client draws again.
- Logout ends the session cleanly, and the session logs no unexpected `CRITICAL` or `ERROR` line.

It does not prove:

- That Gnoblin works on real GPUs or their KMS drivers (Intel, AMD, NVIDIA), with hardware GL or Vulkan, or with
  dma-buf sharing between GPUs. The guest has one virtual GPU with no 3D.
- HDR, colour management, variable refresh rate, fractional scaling or more than one monitor.
- Suspend, resume, hot plug, lid or power events.
- Real input hardware: touchpads, tablets, touch screens, gestures or keyboard layouts other than the default.
- Display manager integration (GDM, SDDM). The test starts the session itself.
- Applications, portals, screen capture, Xwayland or the shell.
- arm64 on real hardware. The arm64 guest is emulated (TCG).

End of life is read from endoflife.date at run time. A release that reaches its end of life stops getting issues on
the next run.
