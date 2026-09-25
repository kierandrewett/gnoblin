# NixOS

The flake includes an experimental Gnoblin path for **x86_64 Linux**. No NixOS
channel has completed the full support gate. See
[platform support](platform-support.md) before installing.

## Stable channels

The default flake package follows Nixpkgs unstable. NixOS 25.05 and 25.11 do
not currently have installable packages: 25.05 lacks `libglycin` and multiple
GNOME 51 dependencies, while 25.11 also lacks several required dependency
floors.

NixOS 26.05 has an experimental package and module. It builds Wayland 1.26 and
the matching scanner privately for Gnoblin's Mutter; neither package replaces
the host Wayland or stock GNOME. The complete pinned Gnoblin package output
builds, including Shell and its runtime closure.

Login, GNOME coexistence, and removal have not passed. Treat this as a test
path, not supported session installation. Its separate module is
`inputs.gnoblin.nixosModules.nixos_26_05` and selects
`inputs.gnoblin.packages.x86_64-linux.gnoblin-nixos-26_05`.

The flake exposes the exact pinned-channel assessment for integrators:

```sh
nix eval --json github:kierandrewett/gnoblin#lib.nixChannelEvaluations.x86_64-linux
```

`lib.nixChannelPackages` names each channel explicitly. For example, the
following reports why 25.11 cannot be installed:

```sh
nix build github:kierandrewett/gnoblin#lib.nixChannelPackages.x86_64-linux.nixos_25_11
```

Each stable entry stops before installation and prints its recorded blockers.
It does not fall back to the unstable package or replace your system libraries.

## 1. Add the flake input

In your existing `flake.nix`:

```nix
inputs.gnoblin.url = "github:kierandrewett/gnoblin";
```

This keeps Gnoblin's own pinned Nixpkgs input. If your system already uses a
compatible unstable revision, you can add
`inputs.gnoblin.inputs.nixpkgs.follows = "nixpkgs";`.

## 2. Enable the module

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

## 3. Rebuild and test the session

From your system configuration directory, run your usual rebuild command:

```sh
sudo nixos-rebuild switch --flake .
```

[Install a shell](bring-your-own-shell.md), then log out and select **Gnoblin**.
The NixOS 26.05 path has not passed login, coexistence or removal, so return to
your existing session if it does not start. Continue with [configuration](/config)
only after it reaches a usable desktop.

## Update or remove

Update the flake input and rebuild to update Gnoblin. Log out and in afterward.

To remove it, remove the Gnoblin module and enable option, then rebuild.
Gnoblin's private runtime does not replace the GNOME packages in your system.
