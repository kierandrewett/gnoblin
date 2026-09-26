# NixOS

The flake includes an experimental Gnoblin path for **x86_64 Linux**. No NixOS
channel has completed the full support gate. See
[platform support](platform-support.md) before installing.

## Stable channels

The default flake package follows the locked rolling Nixpkgs input. The flake
also exposes package outputs and modules for NixOS 25.05 and 25.11. Both use the
same locked private Gnoblin closure because those stable host package sets lack
GNOME 51 dependencies. This leaves the host GNOME Shell and Mutter packages
unchanged.

The stable channel package outputs are `gnoblin-nixos-25_05` and
`gnoblin-nixos-25_11`; the corresponding modules are
`nixosModules.nixos_25_05` and `nixosModules.nixos_25_11`. These are build
targets only: installation, removal, GNOME coexistence, and graphical login
have not passed. The `gnoblin-v0.1.7` tag predates these channel outputs; use a
source ref that includes this change until a later release includes them.

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

Release tags point to the source used by the release build. The tagged NixOS
26.05 package is built in the release workflow before that tag is published.
Pin `inputs.gnoblin.url` to a `gnoblin-v...` release tag when you want a fixed
Gnoblin version. Use a branch or commit containing the stable channel outputs
when testing NixOS 25.05 or 25.11 before a release includes them.

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
The NixOS package paths have not passed login, coexistence or removal, so
return to your existing session if it does not start. Continue with
[configuration](/config) only after it reaches a usable desktop.

## Update or remove

Update the flake input and rebuild to update Gnoblin. Log out and in afterward.

To remove it, remove the Gnoblin module and enable option, then rebuild.
Gnoblin's private runtime does not replace the GNOME packages in your system.
