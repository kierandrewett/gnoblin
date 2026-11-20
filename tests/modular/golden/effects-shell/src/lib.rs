//! Golden shell of example 12 that writes an effect list (`design/modular-framework.md` 5.8 and 12, `docs/abi-compatibility.md`).
//!
//! `GnoblinEffect` is passed as an array, and it grew when the `shader` kind was added. This shell is built against the struct as it
//! was before (152 bytes, no `source`, `params` or `n_params`) and stored as a binary, so running it on a core that has the larger
//! struct shows that the core reads the array at the stride the shell built it with. It gives each window four effects in one list,
//! so a core that indexed the array by its own struct size would read the second record from the middle of the first: it writes the
//! list, and it says what the compositor made of it. A software session draws a shadow and a border and leaves a blur undrawn, so the
//! window's read-only `skipped` list must name the two blurs and nothing else.

use gnoblin_core as gnoblin;
use gnoblin_core::{Effect, EffectKind, Gnoblin, Value, Window};

/// The list every window gets: a shadow, a blur, a border and a second blur, each with values that only that record carries, so a
/// record read from the wrong place is refused or changes the answer.
fn chain() -> Vec<Effect> {
    let mut shadow = Effect::new(EffectKind::Shadow);
    shadow.offset_x = 3.0;
    shadow.offset_y = 5.0;
    shadow.blur_radius = 9.0;
    shadow.spread = 2.0;
    shadow.colour = "#112233cc".to_owned();
    let mut soft = Effect::new(EffectKind::Blur);
    soft.passes = 2;
    soft.tint = "#44556677".to_owned();
    let mut border = Effect::new(EffectKind::Border);
    border.width = 2.5;
    border.offset = 1.0;
    border.inset = true;
    border.colour = "#ff8800".to_owned();
    let mut sharp = Effect::new(EffectKind::Blur);
    sharp.downscale = 4;
    vec![shadow, soft, border, sharp]
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let (major, minor, patch) = gnoblin::installed_abi();
    g.log(&format!(
        "golden-effects-shell: built for ABI {}.{}; installed core ABI {major}.{minor}.{patch}",
        gnoblin::ABI_MAJOR,
        gnoblin::ABI_MINOR
    ));

    g.on_window_property_changed("skipped", |window: &Window, value: &Value| {
        if let Value::Texts(list) = value {
            window.gnoblin().log(&format!(
                "golden-effects-shell: skipped on window {}: [{}]",
                window.id().0,
                list.join("; ")
            ));
        }
    })?;

    let dress = |window: &Window| match window.set_effects(&chain()) {
        Ok(()) => window.gnoblin().log(&format!(
            "golden-effects-shell: wrote 4 effects on window {}",
            window.id().0
        )),
        Err(error) => window.gnoblin().log(&format!(
            "golden-effects-shell: cannot write effects on window {}: {error}",
            window.id().0
        )),
    };
    for window in g.windows()? {
        dress(&window);
    }
    g.on_client_created(dress)?;

    g.log("golden-effects-shell: running");
    Ok(())
}
