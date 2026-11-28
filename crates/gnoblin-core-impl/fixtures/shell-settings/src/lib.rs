//! A core-only shell that writes a setting of the compositor and says what it read before and after.
//!
//! It sets `compositor.user_shaders` to `false` through the core's settings object and calls nothing else: no commit, no reload. The session
//! (`crates/gnoblin/tests/modular/native_settings.rs`) shows that the write reaches the compositor's accepted document and that the next
//! login of the same shell reads it. Every line it logs starts with `settings:`.

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

#[gnoblin::main]
fn main(root: &Gnoblin) -> gnoblin::host::Result<()> {
    let g = root.host();
    let model = root.clone();
    let compositor = model.compositor()?;
    g.log(&format!("settings: before = {}", compositor.user_shaders()?));
    compositor.set_user_shaders(false)?;
    g.log(&format!("settings: after = {}", compositor.user_shaders()?));
    Ok(())
}
