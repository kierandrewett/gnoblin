//! Example 29: the window menu of `libgnoblin-std`.
//!
//! The menu is a card of entries that opens over the windows when a client asks for its menu (`xdg_toplevel.show_window_menu`), holds the
//! keyboard, and answers the arrows, a letter, Return and Escape; a press anywhere else closes it. The library draws and runs all of it. This
//! shell starts the service, gives it three entries of its own (a `menu_provider`) that log what they run, marks the windows of the app
//! `secret` secure (their card is secure too), and logs what the menu does: when a card opens, where the keyboard is, and when it closes.
//! Every line it logs starts with `menus:`.

use gnoblin_std::objects::{MenuEntry, MenuProviderHandlers, WindowMenus};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let menus = WindowMenus::get(&objects)?;

    // The entries: each logs the window it ran on, once the menu has closed. The provider makes them each time a menu opens.
    let made = objects.clone();
    let provider = MenuProviderHandlers {
        items: Some(Box::new(move |_client| {
            ["Alpha", "Beta", "Gamma"]
                .into_iter()
                .map(|label| {
                    let entry = MenuEntry::new(&made, label)?;
                    entry.set_run(move |client| {
                        g.log(&format!("menus: ran {label} on window {client}"));
                        Ok(())
                    })?;
                    Ok(entry)
                })
                .collect()
        })),
        show: None,
    }
    .implement(&objects)?;
    menus.set_provider(&provider)?;
    // The service holds the provider from here on.
    std::mem::forget(provider);

    // A window of the app `secret` is secure: its card is.
    let watching = g.on_client_created(|window| {
        if window.app_id().is_ok_and(|id| id == "secret") {
            let _ = window.set_secure(true);
        }
    })?;
    std::mem::forget(watching);

    menus
        .on_current_changed(move |menu| {
            let client = menu.client().unwrap_or_default();
            let secure = menu
                .panel()
                .ok()
                .flatten()
                .and_then(|card| card.secure().ok())
                .unwrap_or(false);
            g.log(&format!("menus: a card opened for window {client}, secure: {secure}"));
            if let Ok(Some(nav)) = menu.nav() {
                let _ = nav
                    .on_selected_changed(move |row| {
                        g.log(&format!(
                            "menus: the keyboard is on {}",
                            row.label().unwrap_or_default()
                        ));
                    })
                    .map(|listener| listener.keep());
            }
            let _ = menu
                .on_closed(move || g.log("menus: the card closed"))
                .map(|listener| listener.keep());
        })?
        .keep();
    menus.start()?;
    g.log("menus: running");
    Ok(())
}
