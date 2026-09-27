# NixOS

The flake evaluates compatibility for **x86_64 Linux**. It exposes a package
only when the pinned channel supplies the required GNOME development libraries.
If that channel has older desktop schemas, the Nix build uses the pinned GNOME
51 source for that one dependency. No NixOS channel has
completed the full support gate. See
[platform support](platform-support.md) before installing.

## Stable channels

The default flake package follows Nixpkgs unstable when that input is compatible. The
`lib.nixChannelPackages` entries use each named channel and require that
channel's libraries to meet the pinned GNOME source requirements. Desktop
schemas are the one source fallback; the remaining libraries come from Nixpkgs.

The flake exposes the exact pinned-channel assessment for integrators:

```sh
nix eval --json github:kierandrewett/gnoblin#lib.nixChannelEvaluations.x86_64-linux
```

`lib.nixChannelPackages` names each channel explicitly. For example, the
following reports why 25.11 cannot be installed:

```sh
nix build github:kierandrewett/gnoblin#lib.nixChannelPackages.x86_64-linux.nixos_25_11
```

An incompatible entry stops before installation and prints its current blockers.
It does not fall back to the unstable package or replace your system libraries.

## 1. Check compatibility

Run the channel assessment above and check that `buildBlockers` is empty for
your selected channel. If it lists blockers, wait for a channel with the
required package versions before enabling Gnoblin.

## 2. Add the flake input

In your existing `flake.nix`:

```nix
inputs.gnoblin.url = "github:kierandrewett/gnoblin";
```

This keeps Gnoblin's own pinned Nixpkgs input. If your system already uses a
compatible unstable revision, you can add
`inputs.gnoblin.inputs.nixpkgs.follows = "nixpkgs";`.

Release tags point to the source used by the release build.
Pin `inputs.gnoblin.url` to a `gnoblin-v...` release tag when you want a fixed
Gnoblin version.

## 3. Enable the module

Add these entries to your host's `nixosSystem` definition, where `inputs`
is the set of flake inputs:

```nix
modules = [
  inputs.gnoblin.nixosModules.default
  ./configuration.nix
  { programs.gnoblin.enable = true; }
];
```

Keep your existing display manager. If you have none, enable GDM in
`configuration.nix`:

```nix
services.displayManager.gdm.enable = true;
```

## 4. Rebuild and test the session

From your system configuration directory, run your usual rebuild command:

```sh
sudo nixos-rebuild switch --flake .
```

[Install a shell](bring-your-own-shell.md), then log out and select **Gnoblin**.

Run `gnoblin --version` to read the installed Gnoblin release, GNOME component
versions, and source revision; use `gnoblin --version --json` for the full record.
The Git remote may show as `unknown` because Nix does not pass the flake's
original URL to the package build.

The NixOS path has not passed login, coexistence or removal, so return to your
existing session if it does not start. Continue with [configuration](/config)
only after it reaches a usable desktop.

## Update or remove

Update the flake input and rebuild to update Gnoblin. Log out and in afterward.

To remove it, remove the Gnoblin module and enable option, then rebuild.
Gnoblin's private runtime does not replace the GNOME packages in your system.
