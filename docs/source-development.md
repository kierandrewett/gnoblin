# Source development

Start with [build from source](install-source.md). This page covers rebuilding
individual components after that first build.

## Rebuild

Use the private dependency environment when rebuilding a component:

```sh
python3 scripts/build-private-deps.py --run just dev-mutter
python3 scripts/build-private-deps.py --fix-runtime
```

Replace `dev-mutter` with `dev-gnome-shell` or `dev-portal` as
needed. For the complete runtime, use `./build.sh --no-deps`.

Native compositor changes need a fresh session. A config reload does not load
rebuilt libraries.

## Choose a prefix

Default: `./install`, with libraries in `lib64`.

```sh
python3 scripts/build-private-deps.py --prefix /tmp/gnoblin/deps
python3 scripts/build-private-deps.py --prefix /tmp/gnoblin/deps --run \
    env GNOBLIN_PREFIX=/tmp/gnoblin GNOBLIN_LIBDIR=lib just build-source
python3 scripts/build-private-deps.py --prefix /tmp/gnoblin/deps \
    --fix-runtime --runtime-prefix /tmp/gnoblin
```

`GNOBLIN_LIBDIR` is relative to the prefix. Use the same prefix for subsequent
build and devkit commands. System prefixes `/usr` and `/usr/local` are rejected.

## Optional components

These are not part of `./build.sh`. They need additional upstream development
libraries. Build them only when those prerequisites are available:

```sh
python3 scripts/build-private-deps.py --run just dev-portal
python3 scripts/build-private-deps.py --fix-runtime
```

To test the patched portal backend in your test session:

```sh
./install/libexec/xdg-desktop-portal-gnome -r
```

See [permission policy](/config/permissions) before testing remote access.
Old custom remembered-grant files no longer provide approval.

## Verify

For a source build:

```sh
just test-session
```

For a Nix build, use `nix flake check` and `nix build .#gnoblin`. See [testing](testing.md) for the
full suite and [hardware verification](real-hardware-verification.md) for login checks.
