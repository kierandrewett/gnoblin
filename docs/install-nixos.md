# NixOS

Gnoblin provides flake packages and NixOS modules for **x86_64 Linux** on
NixOS 25.05, 25.11, 26.05, and unstable. Each path has passed package build,
declarative enable and removal, and coexistence with the channel's stock GNOME
binaries. Graphical login has not yet been verified. See
[platform support](platform-support.md) before installing.

## Stable channels

The default flake package follows the locked rolling Nixpkgs input. The flake
also exposes package outputs and modules for NixOS 25.05 and 25.11. Both use the
same locked private Gnoblin closure because those stable host package sets lack
GNOME 51 dependencies. This leaves the host GNOME Shell and Mutter packages
unchanged.

The stable channel package outputs are `gnoblin-nixos-25_05` and
`gnoblin-nixos-25_11`; the corresponding modules are
`nixosModules.nixos_25_05` and `nixosModules.nixos_25_11`. The release gate
builds their shared private runtime and verifies that enabling and removing the
module preserves the host GNOME Shell and Mutter binaries. It does not prove a
graphical login.

NixOS 26.05 uses a private Wayland 1.26 build and matching scanner for
Gnoblin's Mutter. Neither replaces the host Wayland or stock GNOME. Its
separate module is
`inputs.gnoblin.nixosModules.nixos_26_05` and selects
`inputs.gnoblin.packages.x86_64-linux.gnoblin-nixos-26_05`.

The flake exposes the exact pinned-channel assessment for integrators:

```sh
nix eval --json github:kierandrewett/gnoblin#lib.nixChannelEvaluations.x86_64-linux
```

`lib.nixChannelPackages` names each channel explicitly. The 25.05 and 25.11
outputs use the private locked closure:

```sh
nix build github:kierandrewett/gnoblin#packages.x86_64-linux.gnoblin-nixos-25_11
```

## 1. Add the flake input

In your existing `flake.nix`:

```nix
inputs.gnoblin.url = "github:kierandrewett/gnoblin";
```

This keeps Gnoblin's own pinned Nixpkgs input. If your system already uses a
compatible unstable revision, you can add
`inputs.gnoblin.inputs.nixpkgs.follows = "nixpkgs";`.

Release tags point to the source used by the release build. Every tagged NixOS
package is built before the release is published.

Gnoblin does not publish a Nix binary cache or a Nix archive. Nix fetches the
tagged flake and builds or substitutes its closure through your configured Nix
stores. Pin
`inputs.gnoblin.url` to a `gnoblin-v...` release tag when you want a fixed
Gnoblin version.

## 2. Enable the module

Add these entries to your host's `nixosSystem` definition, where `inputs`
is the set of flake inputs:

```nix
modules = [
  inputs.gnoblin.nixosModules.nixos_25_11
  ./configuration.nix
  { programs.gnoblin.enable = true; }
];
```

Keep your existing display manager. It must read Wayland session files from
the system profile.

## 3. Rebuild and test the session

From your system configuration directory, run your usual rebuild command:

```sh
sudo nixos-rebuild switch --flake .
```

[Install a shell](bring-your-own-shell.md), then log out and select **Gnoblin**.
The package gates cover declarative install, stock GNOME coexistence, and
removal. Keep another working session available because graphical login still
needs target-hardware verification. Continue with [configuration](/config) only
after it reaches a usable desktop.

## Update or remove

Update the flake input and rebuild to update Gnoblin. Log out and in afterward.

To remove it, remove the Gnoblin module and enable option, then rebuild.
Gnoblin's private runtime does not replace the GNOME packages in your system.
