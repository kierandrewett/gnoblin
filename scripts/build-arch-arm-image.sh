#!/usr/bin/env bash
# Build a bootable, headless aarch64 Arch Linux (Arch Linux ARM) guest image with cloud-init, for the graphical VM test
# of Gnoblin (tests/graphical/harness.py). Arch Linux publishes no aarch64 cloud image, so this makes one.
#
# Output (in --out): NAME (a qcow2 disk), NAME.sha256 and NAME.manifest (what went in: tarball checksum, package list).
# The image has a GPT with an ESP (FAT32, mounted on /boot) and an ext4 root, systemd-boot as the UEFI loader
# (\EFI\BOOT\BOOTAA64.EFI), the mainline linux-aarch64 kernel, a serial console on ttyAMA0, systemd-networkd DHCP, sshd
# and cloud-init with the NoCloud datasource only. No default user and no password: the harness's NoCloud seed makes the
# user and the SSH keys, as on the other distributions.
#
# usage: build-image.sh [--stage STAGE] [--out DIR] [--name FILE] [--cache DIR] [--work DIR]
#   STAGE  all (default) | fetch | extract | prepare | packages | configure | finalize | pack
#     fetch      download the Arch Linux ARM root file system archive (cached) and verify its GPG signature
#     extract    unpack it into WORK/rootfs
#     prepare    files that must exist before the kernel is (re)installed: mkinitcpio hooks
#     packages   pacman -Syu and the guest packages, in a chroot   (needs root and an aarch64 host)
#     configure  boot loader, fstab, cloud-init, services: plain files and symlinks, no chroot
#     finalize   initramfs, clean-up, in a chroot                     (needs root and an aarch64 host)
#     pack       partition table, file systems and the qcow2, with no loop device and no mount
# Run it as root (sudo -E) on an aarch64 host: the chroot runs aarch64 programs natively. `fetch`, `extract`, `prepare`,
# `configure` and `pack` run on any host (as root or under fakeroot).
#
# environment (defaults): ALARM_BASE, DISK_GIB (10), ESP_MIB (512), ALARM_KEY_FILE (alarm-signing-key.asc next to this
# script; the build stops when it is missing: the key is never fetched from the network), GUEST_PACKAGES.
#
# Host packages (Ubuntu 24.04 arm64): arch-install-scripts libarchive-tools gdisk dosfstools mtools e2fsprogs
# qemu-utils gnupg curl systemd (apt-get install -y arch-install-scripts libarchive-tools gdisk dosfstools mtools
# qemu-utils gnupg curl).
#
# Trust: the archive is fetched over plain HTTP from a geo-redirecting mirror, so the only trust anchor is the GPG
# signature, which must be made by the pinned Arch Linux ARM build key. Every package that pacman installs is signed by
# the Arch Linux ARM keyring (SigLevel Required in the archive's pacman.conf).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly HERE
readonly ALARM_BASE="${ALARM_BASE:-http://os.archlinuxarm.org/os}"
readonly ALARM_TARBALL="ArchLinuxARM-aarch64-latest.tar.gz"
readonly ALARM_KEY_FPR="68B3537F39A313B3E574D06777193F152BDBE6A6"
readonly ALARM_KEY_FILE="${ALARM_KEY_FILE:-$HERE/alarm-signing-key.asc}"
readonly DISK_GIB="${DISK_GIB:-10}"
readonly ESP_MIB="${ESP_MIB:-512}"
# cloud-init and its growpart tools (cloud-guest-utils needs sgdisk for GPT), sshd. python comes with cloud-init.
readonly GUEST_PACKAGES="${GUEST_PACKAGES:-cloud-init cloud-guest-utils gptfdisk openssh}"
# Fixed identifiers: the boot entry and fstab name the partitions by PARTUUID, so the stages agree without state.
readonly ESP_PARTUUID="5d1f0b3e-7a52-4c1e-9b0a-2e6f3c0a1001"
readonly ROOT_PARTUUID="5d1f0b3e-7a52-4c1e-9b0a-2e6f3c0a1002"
readonly DISK_GUID="5d1f0b3e-7a52-4c1e-9b0a-2e6f3c0a1000"
readonly ROOT_FS_UUID="5d1f0b3e-7a52-4c1e-9b0a-2e6f3c0a1003"
readonly ESP_VOLID="5D1F0B3E"

STAGE=all
OUT="$PWD/output"
NAME="Arch-Linux-ARM-aarch64-cloudimg.qcow2"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/gnoblin-arch-arm-image"
WORK="$PWD/work"

log() { printf '[arch-arm-image] %s\n' "$*" >&2; }
die() {
    log "error: $*"
    exit 1
}

while (($#)); do
    case "$1" in
        --stage)
            STAGE="$2"
            shift 2
            ;;
        --out)
            OUT="$2"
            shift 2
            ;;
        --name)
            NAME="$2"
            shift 2
            ;;
        --cache)
            CACHE="$2"
            shift 2
            ;;
        --work)
            WORK="$2"
            shift 2
            ;;
        -h | --help)
            sed -n '2,32p' "${BASH_SOURCE[0]}"
            exit 0
            ;;
        *) die "unknown argument: $1" ;;
    esac
done
readonly ROOTFS="$WORK/rootfs"
readonly ESPDIR="$WORK/esp"

need() {
    local missing=()
    for tool in "$@"; do command -v "$tool" >/dev/null || missing+=("$tool"); done
    ((${#missing[@]} == 0)) || die "missing host tools: ${missing[*]}"
}

need_root() { [[ "$(id -u)" == 0 ]] || die "$1 needs root (sudo -E, or fakeroot where the stage allows it)"; }

need_native_chroot() {
    need_root "$1"
    [[ "$(uname -m)" == aarch64 ]] || die "$1 runs aarch64 programs in a chroot: it needs an aarch64 host ($(uname -m) here)"
    need arch-chroot
}

retry() {
    local tries="$1" delay="$2" n=1
    shift 2
    until "$@"; do
        ((n < tries)) || return 1
        log "attempt $n of '$*' failed, retrying in ${delay}s"
        sleep "$delay"
        n=$((n + 1))
    done
}

in_chroot() { arch-chroot "$ROOTFS" /usr/bin/bash -euo pipefail -c "$1"; }

assert_no_mounts() {
    # rm -rf through a leftover bind mount of /dev or /proc would delete the host's files.
    if command -v findmnt >/dev/null && findmnt -rn -o TARGET | grep -q "^$ROOTFS\(/\|$\)"; then
        die "$ROOTFS has mounts below it; unmount them first"
    fi
}

# ---------------------------------------------------------------------------------------------------------------------
stage_fetch() {
    need curl gpg md5sum awk
    mkdir -p "$CACHE"
    local tarball="$CACHE/$ALARM_TARBALL" remote_md5 local_md5=none
    remote_md5="$(retry 5 5 curl -fsSL --max-time 60 "$ALARM_BASE/$ALARM_TARBALL.md5" | awk '{print $1}')"
    [[ "$remote_md5" =~ ^[0-9a-f]{32}$ ]] || die "no usable $ALARM_TARBALL.md5 at $ALARM_BASE"
    [[ -f "$tarball" ]] && local_md5="$(md5sum "$tarball" | awk '{print $1}')"
    if [[ "$local_md5" == "$remote_md5" ]]; then
        log "cached $ALARM_TARBALL ($remote_md5)"
    else
        log "downloading $ALARM_TARBALL (about 830 MB)"
        # The mirror redirects (geo-IP); -C - resumes a partial file across retries.
        retry 5 10 curl -fL --max-time 1800 -C - -o "$tarball.part" "$ALARM_BASE/$ALARM_TARBALL" ||
            die "download of $ALARM_TARBALL failed"
        [[ "$(md5sum "$tarball.part" | awk '{print $1}')" == "$remote_md5" ]] ||
            {
                rm -f "$tarball.part"
                die "downloaded $ALARM_TARBALL does not match its .md5"
            }
        mv "$tarball.part" "$tarball"
    fi
    retry 5 5 curl -fsSL --max-time 60 -o "$tarball.sig" "$ALARM_BASE/$ALARM_TARBALL.sig" || die "no .sig file"
    echo "$remote_md5" >"$tarball.md5"
    verify_signature "$tarball"
}

# The signature must be good and from the pinned key (VALIDSIG ends with the primary key fingerprint), and neither the
# signature nor the key may be expired or revoked. The key comes from the file in the repository only, and whatever
# that file holds, only a signature by the pinned fingerprint passes. The md5 only tells the cache whether the file
# changed; it is not a trust anchor. tests/graphical/arch-arm-image.test.py proves that a tampered archive, a
# tampered signature and another key each fail.
verify_signature() {
    local tarball="$1" home status
    [[ -f "$ALARM_KEY_FILE" ]] || die "no $ALARM_KEY_FILE: the pinned Arch Linux ARM key is part of the repository"
    [[ -f "$tarball.sig" ]] || die "no $tarball.sig"
    home="$(mktemp -d)"
    chmod 700 "$home"
    GNUPGHOME="$home" gpg --batch --quiet --import "$ALARM_KEY_FILE" 2>/dev/null
    status="$(GNUPGHOME="$home" gpg --batch --status-fd 1 --verify "$tarball.sig" "$tarball" 2>/dev/null || true)"
    GNUPGHOME="$home" gpgconf --kill all 2>/dev/null || true
    rm -rf "$home"
    if grep -Eq '^\[GNUPG:\] (EXPSIG|EXPKEYSIG|REVKEYSIG|BADSIG|ERRSIG)' <<<"$status"; then
        die "bad, expired or revoked signature on $ALARM_TARBALL"
    fi
    awk -v fpr="$ALARM_KEY_FPR" '$1 == "[GNUPG:]" && $2 == "VALIDSIG" && $NF == fpr { ok = 1 } END { exit !ok }' \
        <<<"$status" || die "$ALARM_TARBALL is not signed by $ALARM_KEY_FPR"
    log "signature of $ALARM_TARBALL is good (key $ALARM_KEY_FPR)"
}

stage_extract() {
    need bsdtar
    [[ -f "$CACHE/$ALARM_TARBALL" ]] || die "run the fetch stage first"
    # The cached archive is checked again here, so no stage unpacks an archive that was not verified.
    verify_signature "$CACHE/$ALARM_TARBALL"
    need_root extract
    assert_no_mounts
    rm -rf "$ROOTFS" "$ESPDIR"
    mkdir -p "$ROOTFS"
    # bsdtar, not GNU tar: the archive carries extended attributes and ACLs.
    bsdtar -xpf "$CACHE/$ALARM_TARBALL" -C "$ROOTFS" --numeric-owner --xattrs --acls
    mkdir -p "$ROOTFS/boot"
    log "extracted into $ROOTFS ($(du -sm "$ROOTFS" | cut -f1) MiB)"
}

# mkinitcpio's autodetect hook keeps only the modules that the BUILD host uses, and in a chroot that is the runner's
# hardware. The guest needs virtio and whatever else the VM has, so the initramfs is generic (arch-boxes does the same).
#
# pacman's CheckSpace needs the mount point of /var/cache/pacman/pkg, and the root tree is a plain folder, not a mount:
# pacman stops with "could not determine cachedir mount point" and "not enough free disk space" (run 41672380634). The
# image is built without a mount on purpose, so CheckSpace is off while the packages stage runs, and the configure
# stage turns it on again: the guest's pacman checks the space as usual.
readonly CHECKSPACE_OFF="#CheckSpace (off while build-arch-arm-image.sh builds the image)"
stage_prepare() {
    [[ -d "$ROOTFS/etc" ]] || die "run the extract stage first"
    sed -i -E '/^HOOKS=/ s/[[:space:]]autodetect//' "$ROOTFS/etc/mkinitcpio.conf"
    grep -q '^HOOKS=.*autodetect' "$ROOTFS/etc/mkinitcpio.conf" && die "autodetect is still in mkinitcpio.conf"
    log "mkinitcpio hooks: $(grep '^HOOKS=' "$ROOTFS/etc/mkinitcpio.conf")"
    sed -i "s/^CheckSpace\$/$CHECKSPACE_OFF/" "$ROOTFS/etc/pacman.conf"
    grep -q '^CheckSpace' "$ROOTFS/etc/pacman.conf" && die "CheckSpace is still on in pacman.conf"
    log "pacman CheckSpace is off until the configure stage"
}

stage_packages() {
    need_native_chroot packages
    [[ -d "$ROOTFS/usr" ]] || die "run the extract stage first"
    # --disable-sandbox: pacman 7 sandboxes its download user with Landlock and seccomp, which a chroot on a CI kernel
    # may not allow. The archive's pacman has the option (checked in /usr/bin/pacman of the archive).
    local pm="pacman --noconfirm --disable-sandbox"
    retry 3 15 in_chroot "
        pacman-key --init
        pacman-key --populate archlinuxarm
        $pm -Sy archlinuxarm-keyring
        pacman-key --populate archlinuxarm
        $pm -Su --needed $GUEST_PACKAGES
        gpgconf --homedir /etc/pacman.d/gnupg --kill all || true
    " || die "pacman failed in the chroot"
    # The guest has virtio devices only. linux-firmware* is several hundred MiB; nothing in the VM loads it.
    in_chroot "
        fw=\$(pacman -Qq | grep -E '^linux-firmware' || true)
        [[ -z \$fw ]] || pacman -Rdd --noconfirm \$fw
    "
}

write_file() { # write_file PATH [MODE]: the content comes from stdin
    mkdir -p "$(dirname "$ROOTFS$1")"
    cat >"$ROOTFS$1"
    chmod "${2:-0644}" "$ROOTFS$1"
}

stage_configure() {
    [[ -d "$ROOTFS/usr" ]] || die "run the extract stage first"
    need systemctl
    if grep -qxF "$CHECKSPACE_OFF" "$ROOTFS/etc/pacman.conf"; then
        sed -i "s/^$CHECKSPACE_OFF\$/CheckSpace/" "$ROOTFS/etc/pacman.conf"
        grep -qx 'CheckSpace' "$ROOTFS/etc/pacman.conf" || die "CheckSpace is not on again in pacman.conf"
    fi

    # fstab by PARTUUID: the image is copied and overlaid, so labels and device names would be guesses.
    write_file /etc/fstab <<EOF
# <device>                                   <dir>  <type> <options>                          <dump> <pass>
PARTUUID=$ROOT_PARTUUID  /      ext4   rw,relatime                         0      1
PARTUUID=$ESP_PARTUUID  /boot  vfat   rw,relatime,fmask=0077,dmask=0077  0      2
EOF

    # systemd-boot. The firmware's boot variables are empty in a fresh VM, so the loader must sit on the removable-media
    # path \EFI\BOOT\BOOTAA64.EFI. No bootctl: it cannot write EFI variables in a chroot, and a copy does the same job.
    local efi="$ROOTFS/usr/lib/systemd/boot/efi/systemd-bootaa64.efi"
    [[ -f "$efi" ]] || die "no systemd-bootaa64.efi in the image"
    install -D -m 0755 "$efi" "$ROOTFS/boot/EFI/BOOT/BOOTAA64.EFI"
    write_file /boot/loader/loader.conf <<'EOF'
default arch.conf
timeout 0
editor no
console-mode keep
EOF
    # console=tty0 then ttyAMA0: the last console is /dev/console, and the harness reads the PL011 UART (-serial file:).
    # ds=nocloud makes cloud-init use the seed without asking ds-identify to guess. The kernel is EFI-stubbed: sd-boot
    # starts /Image directly. Both files are in the ESP root because the ESP is /boot.
    write_file /boot/loader/entries/arch.conf <<EOF
title   Arch Linux ARM
linux   /Image
initrd  /initramfs-linux.img
options root=PARTUUID=$ROOT_PARTUUID rw rootwait console=tty0 console=ttyAMA0,115200n8 ds=nocloud
EOF

    # cloud-init: NoCloud only (no probing of 169.254.169.254), and no network config of its own, so systemd-networkd
    # keeps the DHCP setup of the archive (en.network, eth.network).
    write_file /etc/cloud/cloud.cfg.d/90-gnoblin-graphical.cfg <<'EOF'
datasource_list: [ NoCloud, None ]
network:
  config: disabled
EOF

    # Units. sshd, systemd-networkd, systemd-resolved and systemd-timesyncd are enabled in the archive already.
    # --root reads the unit files of the image and writes the links there; the host needs no aarch64 code for that.
    systemctl --root="$ROOTFS" enable cloud-init-main.service cloud-init-local.service cloud-init-network.service \
        cloud-config.service cloud-final.service >/dev/null
    # No first-boot prompt on the serial console: systemd-firstboot would ask for a time zone, which is missing.
    systemctl --root="$ROOTFS" mask systemd-firstboot.service >/dev/null
    ln -sf ../usr/share/zoneinfo/UTC "$ROOTFS/etc/localtime"
    log "configured"
}

stage_finalize() {
    need_native_chroot finalize
    assert_no_mounts
    in_chroot "
        # The account of the archive has a known password. The harness logs in by key only: root gets no password
        # hash that works (an invalid hash, not '!', so sshd's own lock check lets a key through), and alarm goes.
        usermod -p '*' root
        userdel -r alarm 2>/dev/null || true
        mkinitcpio -P
        [[ -s /boot/Image && -s /boot/initramfs-linux.img ]]
        rm -rf /var/cache/pacman/pkg/* /var/lib/pacman/sync/* /root/.cache
        rm -f /etc/ssh/ssh_host_*
        : >/etc/machine-id
        gpgconf --homedir /etc/pacman.d/gnupg --kill all || true
    " || die "finalize failed in the chroot"
}

# ---------------------------------------------------------------------------------------------------------------------
# No loop device, no mount: sgdisk writes the table into the file, mkfs.fat and mke2fs write their file systems at the
# partition offsets, mcopy and mke2fs -d fill them. This also runs as an ordinary user under fakeroot.
stage_pack() {
    need sgdisk mkfs.fat mcopy mke2fs qemu-img sha256sum truncate
    [[ -f "$ROOTFS/boot/Image" || -f "$ESPDIR/Image" ]] || die "no kernel in $ROOTFS/boot: run the earlier stages"
    need_root pack
    mkdir -p "$OUT" "$WORK"
    local disk="$WORK/disk.raw"

    # The ESP content leaves the root tree, so that mke2fs -d does not copy it twice (it cannot exclude).
    if [[ -d "$ESPDIR" && -z "$(ls -A "$ROOTFS/boot" 2>/dev/null)" ]]; then
        log "reusing $ESPDIR"
    else
        rm -rf "$ESPDIR"
        mkdir -p "$ESPDIR"
        # dtbs are for boards without UEFI/ACPI; the VM boots from the firmware's own device tree or ACPI.
        rm -rf "$ROOTFS/boot/dtbs" "$ROOTFS/boot/Image.gz" "$ROOTFS/boot/initramfs-linux-fallback.img"
        cp -a "$ROOTFS/boot/." "$ESPDIR/"
        find "$ROOTFS/boot" -mindepth 1 -delete
    fi

    rm -f "$disk"
    truncate -s "${DISK_GIB}G" "$disk"
    # Type 8305 is "Linux root (ARM-64)", ef00 the ESP.
    sgdisk --clear --disk-guid="$DISK_GUID" \
        --new=1:2048:+"${ESP_MIB}M" --typecode=1:ef00 --change-name=1:ESP --partition-guid=1:"$ESP_PARTUUID" \
        --new=2:0:0 --typecode=2:8305 --change-name=2:root --partition-guid=2:"$ROOT_PARTUUID" \
        "$disk" >/dev/null
    local esp_start esp_end root_start root_end
    esp_start="$(sgdisk -i 1 "$disk" | awk '/^First sector:/ {print $3}')"
    esp_end="$(sgdisk -i 1 "$disk" | awk '/^Last sector:/ {print $3}')"
    root_start="$(sgdisk -i 2 "$disk" | awk '/^First sector:/ {print $3}')"
    root_end="$(sgdisk -i 2 "$disk" | awk '/^Last sector:/ {print $3}')"
    [[ -n "$esp_start$esp_end$root_start$root_end" ]] || die "cannot read the partition table"
    local esp_kib=$(((esp_end - esp_start + 1) / 2)) root_kib=$(((root_end - root_start + 1) / 2))

    # -s 8 -S 512: 4 KiB clusters. Without -s, mkfs.fat sizes clusters from the size of the whole 10 GiB file, and the
    # 512 MiB ESP would get fewer than 65525 clusters: firmware reads such a volume as FAT16 and fails to mount it.
    mkfs.fat -F 32 -s 8 -S 512 -n ESP -i "$ESP_VOLID" --offset "$esp_start" "$disk" "$esp_kib" >/dev/null
    MTOOLS_SKIP_CHECK=1 mcopy -i "$disk@@$((esp_start * 512))" -s -n "$ESPDIR"/* ::/

    # root_owner: the root directory belongs to root even when this runs as an ordinary user under fakeroot.
    mke2fs -q -F -t ext4 -L root -U "$ROOT_FS_UUID" -m 1 \
        -E "offset=$((root_start * 512)),root_owner=0:0,nodiscard" -d "$ROOTFS" "$disk" "${root_kib}k"

    log "converting to qcow2"
    rm -f "$OUT/$NAME"
    qemu-img convert -f raw -O qcow2 -c "$disk" "$OUT/$NAME"
    (cd "$OUT" && sha256sum "$NAME" >"$NAME.sha256")
    {
        echo "built: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "script: $(sha256sum "${BASH_SOURCE[0]}" | cut -d' ' -f1)"
        echo "tarball-md5: $(cat "$CACHE/$ALARM_TARBALL.md5" 2>/dev/null || echo unknown)"
        echo "tarball-signing-key: $ALARM_KEY_FPR"
        echo "esp-partuuid: $ESP_PARTUUID"
        echo "root-partuuid: $ROOT_PARTUUID"
        echo "packages:"
        find "$ROOTFS/var/lib/pacman/local" -mindepth 1 -maxdepth 1 -type d -printf '  %f\n' | sort
    } >"$OUT/$NAME.manifest"
    if [[ -n "${SUDO_UID:-}" ]]; then
        # The caller (the harness, as the runner user) reads the image and removes its folders afterwards.
        chown "$SUDO_UID:${SUDO_GID:-$SUDO_UID}" "$OUT/$NAME" "$OUT/$NAME.sha256" "$OUT/$NAME.manifest"
        chown -R "$SUDO_UID:${SUDO_GID:-$SUDO_UID}" "$CACHE"
    fi
    [[ -n "${KEEP_RAW:-}" ]] || rm -f "$disk"
    log "wrote $OUT/$NAME ($(du -m "$OUT/$NAME" | cut -f1) MiB, sha256 $(cut -d' ' -f1 "$OUT/$NAME.sha256"))"
}

case "$STAGE" in
    fetch) stage_fetch ;;
    extract) stage_extract ;;
    prepare) stage_prepare ;;
    packages) stage_packages ;;
    configure) stage_configure ;;
    finalize) stage_finalize ;;
    pack) stage_pack ;;
    all)
        # The work tree belongs to root: leave nothing behind for a caller that is not root (the harness deletes its
        # work folder as the runner user). KEEP_WORK=1 keeps it, for a look at the image.
        [[ -n "${KEEP_WORK:-}" ]] || trap 'assert_no_mounts && rm -rf "$ROOTFS" "$ESPDIR" "$WORK/disk.raw"' EXIT
        stage_fetch
        stage_extract
        stage_prepare
        stage_packages
        stage_configure
        stage_finalize
        stage_pack
        ;;
    *) die "unknown stage: $STAGE" ;;
esac
