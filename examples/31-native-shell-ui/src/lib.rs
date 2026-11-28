//! Example 31: the parts of the stock shell above the toolkit, from `libgnoblin-std`.
//!
//! This shell:
//!
//! - starts the not-responding alert, which looks at the windows and opens a card for one that stops answering the compositor's pings:
//!   Escape waits, Tab moves to Force quit, and Force quit takes a press only after `arm_ms`;
//! - puts the stock lock screen to work: a clock and a password entry over the screen while the session is locked;
//! - opens the launcher with `<Super>space`: Return runs the command typed and closes it, Escape closes it;
//! - flips the overview with `<Super>o`;
//! - shows a toast with `<Super>t`;
//! - calls the toolkit's two functions that need no canvas: `Ui::estimate_text` and `Ui::encode_paint_list`.
//!
//! Every line it logs starts with `shell-ui:`. Build it with `cargo build -p gnoblin-example-native-shell-ui` and run it with
//! `gnoblin ./target/debug/libnative_shell_ui.so`.

use gnoblin_std::{Launcher, LockScreen, NotRespondingAlerts, Overview, PaintKind, PaintOp, Toast, Ui};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;

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

    // A toast fades in for 20 ms, holds for 400 ms and fades out for 20 ms, then removes itself.
    let library = objects.clone();
    g.bind("<Super>t", move || {
        // The toast reads this clock in milliseconds, in place of the toolkit's, so the fade follows the shell's own time.
        let started = std::time::Instant::now();
        let clock = move || Ok(started.elapsed().as_secs_f64() * 1000.0);
        match Toast::show(
            &library,
            "Toast of the shell",
            Some(20.0),
            Some(400.0),
            Some(20.0),
            Some(clock),
        ) {
            Ok(_) => g.log("shell-ui: the toast is up"),
            Err(error) => g.log(&format!("shell-ui: the toast failed: {error}")),
        }
    })?;

    // What a text is taken to measure until it is measured: 5 pixels for each of 5 characters at size 10, a line of 15, a baseline at 11.
    let estimate = Ui::estimate_text(&objects, "Hello", 10.0)?;
    g.log(&format!(
        "shell-ui: estimate {}x{} ascent {}",
        estimate.width, estimate.height, estimate.ascent
    ));

    // The bytes of a paint list: the magic `GNPL`, a version, a count, then the operations. A list with one bad operation is refused whole.
    let mut push = PaintOp::new(PaintKind::PushOpacity);
    push.alpha = Some(0.5);
    // The line's width is its `width`, as it is in the TypeScript library: the bytes carry it as a little-endian f32 after the line's four
    // ends.
    let mut line = PaintOp::new(PaintKind::Line);
    (line.x2, line.y2, line.width) = (Some(5.0), Some(6.0), Some(3.0));
    let list = Ui::encode_paint_list(&objects, &[push, line, PaintOp::new(PaintKind::PopOpacity)])?;
    let line_width = f32::from_le_bytes([list.0[31], list.0[32], list.0[33], list.0[34]]);
    g.log(&format!(
        "shell-ui: paint list of {} bytes {}, line width {line_width}",
        list.0.len(),
        String::from_utf8_lossy(&list.0[..4])
    ));
    let mut bad = PaintOp::new(PaintKind::Rect);
    bad.fill = Some("red".to_owned());
    if let Err(error) = Ui::encode_paint_list(&objects, &[PaintOp::new(PaintKind::PopOpacity), bad]) {
        g.log(&format!("shell-ui: bad paint list refused: {error}"));
    }

    g.log("shell-ui: running");
    Ok(())
}
