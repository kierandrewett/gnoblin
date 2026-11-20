# Build from source

Use this on a system that has no package for Gnoblin. You build Gnoblin from a Git checkout with Cargo, install it into a
prefix with `cargo xtask dist`, and add a login session.

## Prerequisites

Install these with your package manager:

- A C compiler, `pkg-config` and Git
- [`just`](https://just.systems/), optional: the recipes below are one-line wrappers over `cargo` and `cargo xtask`
- Rust, through [rustup](https://rustup.rs/). The toolchain that `rust-toolchain.toml` pins is installed on first use.
- The system libraries the Rust crates link: `libwayland`, `libdrm`, `libinput`, `libseat`, `libudev`, `libgbm`,
  `libxkbcommon`, `pixman`, `PipeWire`, `libclang` and `OpenSSL`, with their development files, and Python's for the optional Python shell runtime

`scripts/ci-install-system-libraries.sh` installs all of them on Ubuntu, Debian and Fedora, and
`scripts/ci-install-system-libraries.sh list` prints the package names without installing anything. Run `cargo xtask
native-libs` to check that the list covers every `-sys` crate in the tree.

At run time you also need Xwayland for X11 applications, `dbus-daemon`, `xdg-desktop-portal` and WirePlumber.

## 1. Get the source and build

```sh
git clone https://github.com/kierandrewett/gnoblin.git
cd gnoblin
cargo build --release --workspace
```

Run the build as your normal user.

## 2. Try it in a window

From a Wayland desktop, run the build tree as a nested session:

```sh
cargo xtask stage --release
GNOBLIN_BACKEND=nested target/release/gnoblin
```

A Gnoblin window opens on your desktop. Close it to end the session. See [backends](backends.md) for the nested
backend and its settings, and [Start Gnoblin and choose a shell](starting-gnoblin.md) for which shell runs.

## 3. Install and add a login session {#login-session}

Install `xdg-desktop-portal` and WirePlumber with your package manager first. Then install the session under a prefix:

```sh
just dist /usr/local
```

`just dist PREFIX` builds the workspace and the XR extension in release mode, then runs
`cargo xtask dist install --prefix PREFIX`. It installs:

- The `gnoblin` and `gnoblinctl` programs, the core and std libraries, and the shell runtimes
- The portal backend, the PAM module and the lock screen's PAM service
- The **Gnoblin** login entry, `share/wayland-sessions/gnoblin.desktop`, and the systemd user target

[Install layout](install-layout.md) lists every file. A login manager finds the login entry only in a directory it
searches, and `/usr` is always one; pass `--prefix /usr` to `cargo xtask dist install` for a system-wide session, as root
or with `--destdir` for a staged copy.

[Install a desktop shell and log in](installation.md#after-installing). Run `gnoblin --version` to check the installed
version.

## Check the install

```sh
cargo xtask dist-check --prefix /usr/local
```

It verifies the core library's name and exports, what each library links against, and that no library is missing.

## Update

```sh
git pull --ff-only
just dist /usr/local
```

`cargo xtask dist install` replaces only files its manifest lists and that still hold what the install wrote, and refuses
any other file at a destination before it writes anything. Log out and back in to use the new version.

## Remove

Log in to another session first. Then run:

```sh
cargo xtask dist uninstall --prefix /usr/local
```

It removes exactly the files `share/gnoblin/install-manifest` lists and that still hold what the install wrote, keeps and
reports a file you changed, and removes a directory only when it is empty. Your shell and your configuration stay.

See [source development](source-development.md) to work on the code.
