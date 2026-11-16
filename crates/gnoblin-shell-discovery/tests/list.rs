//! `--list-shells`: the row model and the table.

mod support;

use std::path::PathBuf;

use gnoblin_shell_discovery::{DiscoveryError, Language, Request, Tier, list_shells};
use support::Sandbox;

fn table(sandbox: &Sandbox, request: &Request) -> String {
    sandbox.normalized(&list_shells(&sandbox.environment, request).render())
}

#[test]
fn an_empty_system_lists_only_the_header() {
    let sandbox = Sandbox::new();
    let listing = list_shells(&sandbox.environment, &Request::Discover);
    assert!(listing.rows.is_empty());
    assert!(matches!(listing.selected, Err(DiscoveryError::NoShell { .. })));
    assert_eq!(listing.render(), "NAME  TIER  LANGUAGE  DEFAULT  LOCATION");
}

#[test]
fn rows_come_in_precedence_order_with_the_selected_one_marked() {
    let sandbox = Sandbox::new();
    let module = sandbox.touch("opt/tiler/libtiler.so");
    sandbox.write(
        "home/.local/share/gnoblin/shells/tiler.toml",
        &format!("path = \"{}\"\n", module.display()),
    );
    sandbox.write("home/.config/gnoblin/config.toml", "[shell]\ndefault = \"tiler\"\n");
    sandbox.touch("usr/share/gnoblin/shells/lines.py");
    sandbox.touch("prefix/share/gnoblin/default-shell/shell.ts");
    let expected = "NAME     TIER  LANGUAGE  DEFAULT  LOCATION
tiler    5     native    yes      <root>/home/.local/share/gnoblin/shells/tiler.toml -> <root>/opt/tiler/libtiler.so
lines    6     python             <root>/usr/share/gnoblin/shells/lines.py
default  7     ts                 <root>/prefix/share/gnoblin/default-shell/shell.ts  (packaged)";
    assert_eq!(table(&sandbox, &Request::Discover), expected);
}

#[test]
fn a_shell_hidden_by_an_earlier_tier_is_marked_shadowed() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/default.py");
    sandbox.touch("prefix/share/gnoblin/default-shell/shell.ts");
    let expected = "NAME     TIER  LANGUAGE  DEFAULT   LOCATION
default  5     python    yes       <root>/home/.local/share/gnoblin/shells/default.py
default  7     ts        shadowed  <root>/prefix/share/gnoblin/default-shell/shell.ts  (packaged)";
    assert_eq!(table(&sandbox, &Request::Discover), expected);
    let listing = list_shells(&sandbox.environment, &Request::Discover);
    assert_eq!(
        listing
            .rows
            .iter()
            .map(|row| (row.tier, row.is_default, row.shadowed))
            .collect::<Vec<_>>(),
        vec![(Tier::UserShells, true, false), (Tier::Packaged, false, true)]
    );
}

#[test]
fn conventional_user_files_are_listed_under_tier_four() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.ts");
    let listing = list_shells(&sandbox.environment, &Request::Discover);
    assert_eq!(listing.rows.len(), 1);
    let row = &listing.rows[0];
    assert_eq!(
        (row.name.as_str(), row.tier, row.is_default),
        ("shell", Tier::UserConfig, true)
    );
    assert_eq!(row.language, Some(Language::TypeScript));
}

#[test]
fn an_ambiguous_listing_has_no_default_and_keeps_the_error() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/a.so");
    sandbox.touch("home/.local/share/gnoblin/shells/b.so");
    let listing = list_shells(&sandbox.environment, &Request::Discover);
    assert_eq!(listing.rows.len(), 2);
    assert!(listing.rows.iter().all(|row| !row.is_default));
    assert!(matches!(listing.selected, Err(DiscoveryError::Ambiguous { .. })));
}

#[test]
fn an_unusable_shell_is_listed_with_its_problem() {
    let sandbox = Sandbox::new();
    sandbox.write("usr/share/gnoblin/shells/tiler.toml", "path = \"/opt/gone/shell.py\"\n");
    let expected = "NAME   TIER  LANGUAGE  DEFAULT  LOCATION
tiler  6     ?                  <root>/usr/share/gnoblin/shells/tiler.toml  [unusable: its target /opt/gone/shell.py does not exist]";
    assert_eq!(table(&sandbox, &Request::Discover), expected);
}

#[test]
fn a_directory_shell_shows_the_file_it_leads_to() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/mine/shell.js");
    let expected = "NAME  TIER  LANGUAGE  DEFAULT  LOCATION
mine  5     js        yes      <root>/home/.local/share/gnoblin/shells/mine -> <root>/home/.local/share/gnoblin/shells/mine/shell.js";
    assert_eq!(table(&sandbox, &Request::Discover), expected);
}

#[test]
fn explicit_requests_are_listed_first_and_win_the_marker() {
    let mut sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/installed.so");
    sandbox.touch("work/mine.py");
    let listing = list_shells(&sandbox.environment, &Request::Path(PathBuf::from("mine.py")));
    let rows: Vec<_> = listing
        .rows
        .iter()
        .map(|row| (row.name.as_str(), row.tier, row.is_default))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("mine", Tier::CommandLine, true),
            ("installed", Tier::UserShells, false)
        ]
    );

    sandbox.environment.gnoblin_shell = Some(sandbox.at("work/mine.py").into_os_string());
    let listing = list_shells(&sandbox.environment, &Request::Discover);
    let rows: Vec<_> = listing
        .rows
        .iter()
        .map(|row| (row.name.as_str(), row.tier, row.is_default))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("mine", Tier::Environment, true),
            ("installed", Tier::UserShells, false)
        ]
    );
}

#[test]
fn a_config_path_is_listed_under_tier_three() {
    let sandbox = Sandbox::new();
    let script = sandbox.touch("opt/tiler/shell.py");
    sandbox.write(
        "home/.config/gnoblin/config.toml",
        &format!("[shell]\npath = \"{}\"\n", script.display()),
    );
    let listing = list_shells(&sandbox.environment, &Request::Discover);
    assert_eq!(listing.rows.len(), 1);
    assert_eq!((listing.rows[0].tier, listing.rows[0].is_default), (Tier::Config, true));
}

#[test]
fn a_named_request_marks_the_installed_shell_it_resolved_to() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/a.so");
    sandbox.touch("home/.local/share/gnoblin/shells/b.so");
    let listing = list_shells(&sandbox.environment, &Request::Name("b".to_owned()));
    let defaults: Vec<&str> = listing
        .rows
        .iter()
        .filter(|row| row.is_default)
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(defaults, vec!["b"]);
}

#[test]
fn config_warnings_travel_with_the_listing() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.js");
    sandbox.write("home/.config/gnoblin/config.toml", "[shell]\ncolour = 1\n");
    let listing = list_shells(&sandbox.environment, &Request::Discover);
    assert_eq!(listing.warnings.len(), 1);
    assert_eq!(listing.warnings[0].message, "unknown key `shell.colour`");
}

#[test]
fn a_broken_config_does_not_stop_the_listing() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/a.so");
    sandbox.write("home/.config/gnoblin/config.toml", "[shell\n");
    let listing = list_shells(&sandbox.environment, &Request::Discover);
    assert_eq!(listing.rows.len(), 1);
    assert!(matches!(listing.selected, Err(DiscoveryError::Config(_))));
}
