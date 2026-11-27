//! `gnoblinctl shell list`: the discovery table of the installed shells, from the real binary.
//!
//! The shell sits in `$XDG_DATA_HOME/gnoblin/shells` (tier 5) and `[shell] default` names it, so the table must show it
//! as native, tier 5, with the default marker and its file as the location. Only private temporary directories are used.

use std::fs;
use std::process::Command;

#[test]
fn shell_list_names_an_installed_shell_with_its_tier_loader_location_and_default_marker() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let data = root.path().join("data");
    let config = root.path().join("config");
    let shells = data.join("gnoblin/shells");
    fs::create_dir_all(&shells).expect("the shells directory");
    fs::create_dir_all(config.join("gnoblin")).expect("the config directory");
    let shell = shells.join("demo.so");
    fs::write(&shell, b"").expect("the shell file");
    fs::write(config.join("gnoblin/config.toml"), "[shell]\ndefault = \"demo\"\n").expect("the config");

    let output = Command::new(env!("CARGO_BIN_EXE_gnoblinctl"))
        .args(["shell", "list"])
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .env("HOME", root.path())
        .env_remove("GNOBLIN_SHELL")
        .output()
        .expect("gnoblinctl runs");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success(), "exit {:?}: {stdout}", output.status.code());

    let header: Vec<&str> = stdout
        .lines()
        .next()
        .expect("a header line")
        .split_whitespace()
        .collect();
    assert_eq!(header, ["NAME", "TIER", "LANGUAGE", "DEFAULT", "LOCATION"], "{stdout}");
    let row: Vec<&str> = stdout
        .lines()
        .find(|line| line.split_whitespace().next() == Some("demo"))
        .unwrap_or_else(|| panic!("a row for demo: {stdout}"))
        .split_whitespace()
        .collect();
    assert_eq!(
        row[1..4],
        ["5", "native", "yes"],
        "tier, loader and default marker: {stdout}"
    );
    assert_eq!(
        row[4],
        shell.display().to_string(),
        "the location is the file: {stdout}"
    );
}
