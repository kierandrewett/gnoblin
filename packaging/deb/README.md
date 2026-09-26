# Debian and Ubuntu packages

Build one `gnoblin` package per target: Debian 11, 12 and 13, Ubuntu 22.04,
24.04 and 26.04. Each package contains the compositor, session and private
runtime at `/usr/lib/gnoblin`. It does not replace GNOME packages.

## Build in a container

Use Docker or Podman. The example below uses Debian 13; substitute one of the
other target images to build for that release. Debian 11 builds use the final
Debian package snapshot from August 31, 2026.

From a clean Gnoblin checkout:

```sh
podman run --name gnoblin-deb-build --security-opt label=disable \
    -v "$PWD:/source:ro" -it debian:13 bash
```

Inside that container:

```sh
apt-get update
apt-get install -y --no-install-recommends git ca-certificates
git clone --no-local /source /build
cd /build
scripts/provision-deb-container.sh
chown -R builder /build
runuser -u builder -- env PATH="/opt/gnoblin-build-tools/bin:$PATH" \
    scripts/build-deb.sh
exit
```

For Debian 11/12 and Ubuntu 22.04, use `scripts/provision-deb-compat-container.sh`
and build with `PATH="/opt/gnoblin-compat-build-tools/bin:$PATH"
scripts/build-deb-compat-runtime.sh`. That path builds the newer runtime
libraries privately because those hosts do not provide the GNOME 51 interfaces
the package needs.

Copy the result back to your host:

```sh
podman cp gnoblin-deb-build:/build/dist/deb ./deb-packages
```

The scripts install build tools only inside the container. The build itself
runs without root privileges. `GNOBLIN_BUILD_JOBS=2` limits memory use on smaller
builders. Pass `--revision 2` to `build-deb.sh` for a second packaging revision
of the same GNOME release.

## What the package contains

The shared dependency builder reads `build-dependencies.json`. Debian 13 and
Ubuntu 24.04 build the libraries in `packaging/deb/build-dependencies.json`,
including SpiderMonkey and Glycin. Debian 11/12 and Ubuntu 22.04 use a pinned
compatibility closure for GLib, Wayland, Pango, GTK, GCR, SpiderMonkey, GJS and Glycin;
those libraries stay under `/usr/lib/gnoblin/deps` too. A checksum-verified
Rust 1.85.1 toolchain is used only while building the older targets.

The package exports only its login entry, `gnoblinctl`, Gnoblin user units and
separately named backlight policy. Headers and static libraries are omitted.
`dpkg-shlibdeps` determines the remaining system-library dependencies from the
binaries. It must resolve every dependency; missing library metadata is an error.

`/usr/share/doc/gnoblin/build-info.json` records the source commit and target
system. The package also includes the dependency manifest and licence notices.

## Test the installed package

Use a **fresh container of the same distribution**. Mount this checkout read-only
and copy in the `.deb`, then run as root:

```sh
/source/scripts/test-deb.sh /tmp/gnoblin-debian13-amd64.deb
```

The test installs stock GNOME first, installs Gnoblin, starts a headless desktop,
uses the installed CLI to manage a window, checks protocol isolation, and removes
Gnoblin. Stock GNOME's binary and version must remain unchanged throughout.

This does not exercise a display manager or a hardware seat. Follow
[hardware verification](../../docs/real-hardware-verification.md) before declaring
a release ready for normal desktop use.

## Release automation

`.github/workflows/deb.yml` builds and tests all six targets on pushes,
pull requests and release builds. Release builds use the exact tag revision.
The release workflow waits for every target before publishing `.deb` files,
checksums and dependency source archives to GitHub Releases. It then publishes
the `.deb` files to the signed Gnoblin APT archive on GitHub Pages. The archive
retains each distribution's packages separately and uses the public key in
`packaging/apt/gnoblin-archive-keyring.asc`.
