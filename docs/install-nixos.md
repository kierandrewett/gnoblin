# NixOS

The flake provides a Gnoblin session for **x86_64 Linux**.
Use Nixpkgs unstable for the current GNOME dependency stack.

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

## 3. Rebuild and log in

From your system configuration directory, run your usual rebuild command:

```sh
sudo nixos-rebuild switch --flake .
```

[Install a shell](bring-your-own-shell.md), then log out and select **Gnoblin**.
Continue with [configuration](/config).

## Update or remove

Update the flake input and rebuild to update Gnoblin. Log out and in afterward.

To remove it, remove the Gnoblin module and enable option, then rebuild.
Gnoblin's private runtime does not replace the GNOME packages in your system.
