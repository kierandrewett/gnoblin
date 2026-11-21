//! Example 31: the parts of the stock shell above the toolkit, from `libgnoblin-std`.
//!
//! This shell:
//!
//! - starts the not-responding alert, which looks at the windows and opens a card for one that stops answering the compositor's pings:
//!   Escape waits, Tab moves to Force quit, and Force quit takes a press only after `arm_ms`;
//! - puts the stock lock screen to work: a clock and a password entry over the screen while the session is locked;
//! - opens the launcher with `<Super>space`: Return runs the command typed and closes it, Escape closes it;
//! - flips the overview with `<Super>o`.
//!
//! Every line it logs starts with `shell-ui:`. Build it with `cargo build -p gnoblin-example-native-shell-ui` and run it with
//! `gnoblin ./target/debug/libnative_shell_ui.so`.

use gnoblin_std::objects::{Launcher, LockScreen, NotRespondingAlerts, Overview};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;

    // A long delay, so that a Return pressed at once finds Force quit still off.
    let alerts = NotRespondingAlerts::get(&objects)?;
    alerts.set_arm_ms(3000.0)?;
    alerts
        .on_answered(move |client, _answer| g.log(&format!("shell-ui: the alert for window {client} ended")))?
        .keep();
    alerts.start()?;

    // Making the screen puts it to work for the generation: it is up while the session is locked. It is retained, so it stays when the
    // shell lets go of its reference.
    let lock = LockScreen::new(&objects, &LockScreen::default_lock_style(&objects)?)?;
    g.log(&format!("shell-ui: the lock screen is ready, shown {}", lock.shown()?));

    let overview = Overview::get(&objects)?;
    g.bind("<Super>o", move || match overview.toggle() {
        Ok(on) => g.log(&format!("shell-ui: the overview is {}", if on { "on" } else { "off" })),
        Err(error) => g.log(&format!("shell-ui: the overview failed: {error}")),
    })?;

    let library = objects.clone();
    g.bind("<Super>space", move || {
        let opened = Launcher::open(
            &library,
            Some(move |command: String| {
                g.log(&format!("shell-ui: the launcher ran {command}"));
                Ok(())
            }),
        );
        match opened {
            Ok(_) => g.log("shell-ui: the launcher is open"),
            Err(error) => g.log(&format!("shell-ui: the launcher failed: {error}")),
        }
    })?;

    g.log("shell-ui: running");
    Ok(())
}
