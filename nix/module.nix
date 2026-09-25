{
  self,
  defaultPackage ? (system: self.packages.${system}.gnoblin),
  defaultPackageText ? "inputs.gnoblin.packages.\${pkgs.stdenv.hostPlatform.system}.gnoblin",
}:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.gnoblin;
  system = pkgs.stdenv.hostPlatform.system;
in
{
  options.programs.gnoblin = {
    enable = lib.mkEnableOption "the Gnoblin display-manager session";

    package = lib.mkOption {
      type = lib.types.package;
      default = defaultPackage system;
      defaultText = lib.literalExpression defaultPackageText;
      description = "The Gnoblin package that provides the session, systemd user units, and control tools.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    services.displayManager.sessionPackages = [ cfg.package ];
    systemd.packages = [ cfg.package ];
  };
}
