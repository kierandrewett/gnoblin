//! The checked-in routing must never send `org.freedesktop.impl.portal.Secret` to a backend
//! that holds the user's keyring.
//!
//! A `Secret` request asks for the sandbox master key. `xdg-desktop-portal` picks the backend
//! from `portals.conf`; a `*` or a fallback name lets it pick any installed implementation,
//! including gnome-keyring's `org.freedesktop.secrets`. Gnoblin does not implement `Secret`, so
//! the only route it may have is `gnoblin` alone: the frontend then finds no implementation and
//! the caller is refused, rather than reaching another keyring.

const PORTALS_CONF: &str = include_str!("../data/portals.conf");
const GNOBLIN_PORTAL: &str = include_str!("../data/gnoblin.portal");
const SECRET: &str = "org.freedesktop.impl.portal.Secret";

/// The `interface=backends` lines of `portals.conf`, comments and section headers skipped.
fn routes(conf: &str) -> Vec<(&str, &str)> {
    conf.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('['))
        .filter_map(|line| line.split_once('='))
        .collect()
}

/// The value of `Interfaces=` in `gnoblin.portal`, split into interface names.
fn gnoblin_interfaces(portal: &str) -> Vec<&str> {
    portal
        .lines()
        .filter_map(|line| line.strip_prefix("Interfaces="))
        .flat_map(|list| list.split(';'))
        .filter(|name| !name.is_empty())
        .collect()
}

#[test]
fn secret_is_routed_to_gnoblin_alone() {
    let secret = routes(PORTALS_CONF)
        .into_iter()
        .filter(|(interface, _)| *interface == SECRET)
        .map(|(_, backends)| backends)
        .collect::<Vec<_>>();
    assert_eq!(
        secret,
        vec!["gnoblin"],
        "Secret must route to gnoblin only: any `*` or fallback lets xdg-desktop-portal pick a keyring"
    );
}

#[test]
fn gnoblin_does_not_claim_secret() {
    // The route above is only safe while Gnoblin does not list `Secret`: then the frontend
    // skips it and finds no implementation. Listing it would make Gnoblin answer a request it has
    // no store rule for.
    assert!(
        !gnoblin_interfaces(GNOBLIN_PORTAL).contains(&SECRET),
        "gnoblin.portal must not list {SECRET}"
    );
}
