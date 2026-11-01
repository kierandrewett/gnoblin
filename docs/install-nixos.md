# NixOS

Gnoblin is available as a flake for NixOS on x86_64. It is experimental and needs NixOS unstable. Stable channels are
not supported yet.

## Install

1. Add Gnoblin to the `inputs` of your `flake.nix`:

    ```nix
    inputs.gnoblin.url = "github:kierandrewett/gnoblin";
    ```

    To stay on one release, use a release tag instead, for example `github:kierandrewett/gnoblin/gnoblin-v0.1.13`.

2. Add the module to your host's `nixosSystem` definition, where `inputs` is the set of flake inputs:

    ```nix
    modules = [
      inputs.gnoblin.nixosModules.default
      ./configuration.nix
      { programs.gnoblin.enable = true; }
    ];
    ```

3. Keep your display manager. If you have none, enable GDM in `configuration.nix`:

    ```nix
    services.displayManager.gdm.enable = true;
    ```

4. Rebuild from your system configuration directory:

    ```sh
    sudo nixos-rebuild switch --flake .
    ```

## Next steps

[Install a desktop shell and log in](installation.md#after-installing).

## Update

Update the flake input, then rebuild:

```sh
nix flake update gnoblin
sudo nixos-rebuild switch --flake .
```

Log out and back in to use the new version.

## Remove

Remove the Gnoblin module and the `programs.gnoblin.enable` line, then rebuild. Your shell and your configuration
stay.

See [troubleshooting](troubleshooting.md) if the session does not start.
