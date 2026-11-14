//! The stock library: the TypeScript every generation can import as the embedded `gnoblin` modules.
//!
//! The sources live in `data/js/gnoblin/` and are compiled into the binary. [`stock_modules`] strips their
//! types once and returns the name to JavaScript map [`JsConfig::embed`](crate::JsConfig::embed) takes. The
//! modules import one another by their embedded names (`gnoblin/layout`), because the loader resolves a
//! relative import only against a file module.

use std::collections::HashMap;
use std::path::Path;

use crate::ts::strip_types;

/// The `init.ts` `gnoblinctl config init` writes (`data/js/starter/init.ts`). As written it is the stock default
/// config; the examples in it are comments to switch on.
pub const STARTER_CONFIG: &str = include_str!("../../../data/js/starter/init.ts");

/// `(import name, file under data/js/gnoblin, source)`. `gnoblin` is the user-facing module and only
/// re-exports the others.
const MODULES: [(&str, &str, &str); 54] = [
    ("gnoblin", "index.ts", include_str!("../../../data/js/gnoblin/index.ts")),
    (
        "gnoblin/bindings",
        "bindings.ts",
        include_str!("../../../data/js/gnoblin/bindings.ts"),
    ),
    (
        "gnoblin/devices",
        "devices.ts",
        include_str!("../../../data/js/gnoblin/devices.ts"),
    ),
    (
        "gnoblin/input",
        "input.ts",
        include_str!("../../../data/js/gnoblin/input.ts"),
    ),
    (
        "gnoblin/input_list",
        "input_list.ts",
        include_str!("../../../data/js/gnoblin/input_list.ts"),
    ),
    (
        "gnoblin/native_object",
        "native_object.ts",
        include_str!("../../../data/js/gnoblin/native_object.ts"),
    ),
    (
        "gnoblin/layout",
        "layout.ts",
        include_str!("../../../data/js/gnoblin/layout.ts"),
    ),
    (
        "gnoblin/layouts/floating",
        "layouts/floating.ts",
        include_str!("../../../data/js/gnoblin/layouts/floating.ts"),
    ),
    (
        "gnoblin/layouts/grid",
        "layouts/grid.ts",
        include_str!("../../../data/js/gnoblin/layouts/grid.ts"),
    ),
    (
        "gnoblin/layouts/master-stack",
        "layouts/master-stack.ts",
        include_str!("../../../data/js/gnoblin/layouts/master-stack.ts"),
    ),
    (
        "gnoblin/layouts/monocle",
        "layouts/monocle.ts",
        include_str!("../../../data/js/gnoblin/layouts/monocle.ts"),
    ),
    (
        "gnoblin/layouts/dwindle",
        "layouts/dwindle.ts",
        include_str!("../../../data/js/gnoblin/layouts/dwindle.ts"),
    ),
    (
        "gnoblin/layouts/kiosk",
        "layouts/kiosk.ts",
        include_str!("../../../data/js/gnoblin/layouts/kiosk.ts"),
    ),
    (
        "gnoblin/layouts/strip",
        "layouts/strip.ts",
        include_str!("../../../data/js/gnoblin/layouts/strip.ts"),
    ),
    (
        "gnoblin/sets",
        "sets.ts",
        include_str!("../../../data/js/gnoblin/sets.ts"),
    ),
    (
        "gnoblin/gestures",
        "gestures.ts",
        include_str!("../../../data/js/gnoblin/gestures.ts"),
    ),
    (
        "gnoblin/workspace",
        "workspace.ts",
        include_str!("../../../data/js/gnoblin/workspace.ts"),
    ),
    (
        "gnoblin/config",
        "config.ts",
        include_str!("../../../data/js/gnoblin/config.ts"),
    ),
    (
        "gnoblin/dialogs",
        "dialogs.ts",
        include_str!("../../../data/js/gnoblin/dialogs.ts"),
    ),
    (
        "gnoblin/authentication",
        "authentication.ts",
        include_str!("../../../data/js/gnoblin/authentication.ts"),
    ),
    (
        "gnoblin/idle",
        "idle.ts",
        include_str!("../../../data/js/gnoblin/idle.ts"),
    ),
    (
        "gnoblin/launches",
        "launches.ts",
        include_str!("../../../data/js/gnoblin/launches.ts"),
    ),
    (
        "gnoblin/titlebar",
        "titlebar.ts",
        include_str!("../../../data/js/gnoblin/titlebar.ts"),
    ),
    ("gnoblin/ui", "ui.ts", include_str!("../../../data/js/gnoblin/ui.ts")),
    (
        "gnoblin/focus",
        "focus.ts",
        include_str!("../../../data/js/gnoblin/focus.ts"),
    ),
    (
        "gnoblin/rules",
        "rules.ts",
        include_str!("../../../data/js/gnoblin/rules.ts"),
    ),
    (
        "gnoblin/snap",
        "snap.ts",
        include_str!("../../../data/js/gnoblin/snap.ts"),
    ),
    (
        "gnoblin/transition",
        "transition.ts",
        include_str!("../../../data/js/gnoblin/transition.ts"),
    ),
    (
        "gnoblin/widgets",
        "widgets.ts",
        include_str!("../../../data/js/gnoblin/widgets.ts"),
    ),
    (
        "gnoblin/menus",
        "menus.ts",
        include_str!("../../../data/js/gnoblin/menus.ts"),
    ),
    (
        "gnoblin/alerts",
        "alerts.ts",
        include_str!("../../../data/js/gnoblin/alerts.ts"),
    ),
    (
        "gnoblin/launcher",
        "launcher.ts",
        include_str!("../../../data/js/gnoblin/launcher.ts"),
    ),
    (
        "gnoblin/overview",
        "overview.ts",
        include_str!("../../../data/js/gnoblin/overview.ts"),
    ),
    (
        "gnoblin/panel",
        "panel.ts",
        include_str!("../../../data/js/gnoblin/panel.ts"),
    ),
    (
        "gnoblin/style",
        "style.ts",
        include_str!("../../../data/js/gnoblin/style.ts"),
    ),
    (
        "gnoblin/inspect",
        "inspect.ts",
        include_str!("../../../data/js/gnoblin/inspect.ts"),
    ),
    (
        "gnoblin/scene",
        "scene.ts",
        include_str!("../../../data/js/gnoblin/scene.ts"),
    ),
    (
        "gnoblin/display",
        "display.ts",
        include_str!("../../../data/js/gnoblin/display.ts"),
    ),
    (
        "gnoblin/display_manager",
        "display_manager.ts",
        include_str!("../../../data/js/gnoblin/display_manager.ts"),
    ),
    ("gnoblin/xr", "xr.ts", include_str!("../../../data/js/gnoblin/xr.ts")),
    (
        "gnoblin/xr_bindings",
        "xr_bindings.ts",
        include_str!("../../../data/js/gnoblin/xr_bindings.ts"),
    ),
    (
        "gnoblin/virtual_output",
        "virtual_output.ts",
        include_str!("../../../data/js/gnoblin/virtual_output.ts"),
    ),
    (
        "gnoblin/extensions/xr",
        "extensions/xr.ts",
        include_str!("../../../data/js/gnoblin/extensions/xr.ts"),
    ),
    (
        "gnoblin/window_state",
        "window_state.ts",
        include_str!("../../../data/js/gnoblin/window_state.ts"),
    ),
    (
        "gnoblin/operation",
        "operation.ts",
        include_str!("../../../data/js/gnoblin/operation.ts"),
    ),
    (
        "gnoblin/process",
        "process.ts",
        include_str!("../../../data/js/gnoblin/process.ts"),
    ),
    (
        "gnoblin/session",
        "session.ts",
        include_str!("../../../data/js/gnoblin/session.ts"),
    ),
    (
        "gnoblin/reposition",
        "reposition.ts",
        include_str!("../../../data/js/gnoblin/reposition.ts"),
    ),
    (
        "gnoblin/gesture_actions",
        "gesture_actions.ts",
        include_str!("../../../data/js/gnoblin/gesture_actions.ts"),
    ),
    (
        "gnoblin/lock",
        "lock.ts",
        include_str!("../../../data/js/gnoblin/lock.ts"),
    ),
    (
        "gnoblin/spatial",
        "spatial.ts",
        include_str!("../../../data/js/gnoblin/spatial.ts"),
    ),
    (
        "gnoblin/properties",
        "properties.ts",
        include_str!("../../../data/js/gnoblin/properties.ts"),
    ),
    (
        "gnoblin/scene_mirror",
        "scene_mirror.ts",
        include_str!("../../../data/js/gnoblin/scene_mirror.ts"),
    ),
    (
        "gnoblin/layouts/arc",
        "layouts/arc.ts",
        include_str!("../../../data/js/gnoblin/layouts/arc.ts"),
    ),
];

/// The stock modules by import name, as JavaScript, ready to embed in a [`JsConfig`](crate::JsConfig).
///
/// A source that fails to strip cannot happen for the files compiled in (a test imports every one), but a
/// panic in a compositor-side process is worse than an error where it is used, so such a module becomes
/// one that throws the reason when imported.
pub fn stock_modules() -> HashMap<String, String> {
    stock_modules_in(None)
}

/// Like [`stock_modules`], reading each module from `dir` (laid out as `data/js/gnoblin`) when the file is there.
/// A file that is missing, or that cannot be read, is the compiled-in module: a half-installed directory must
/// not leave the shell without a stock library. A file that does not strip becomes a module that throws.
pub fn stock_modules_in(dir: Option<&Path>) -> HashMap<String, String> {
    MODULES
        .iter()
        .map(|(name, file, compiled)| {
            let path = dir.map_or_else(|| Path::new("data/js/gnoblin").join(file), |dir| dir.join(file));
            let from_dir = dir.and_then(|_| std::fs::read_to_string(&path).ok());
            let source = from_dir.as_deref().unwrap_or(compiled);
            let javascript = match strip_types(&path, source) {
                Ok(stripped) => stripped.code,
                Err(error) => {
                    let reason = error.to_string().replace(['\\', '"', '\n'], " ");
                    format!("throw new Error(\"stock module {name} does not compile: {reason}\");\n")
                }
            };
            ((*name).to_owned(), javascript)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stock_module_strips_to_javascript() {
        let modules = stock_modules();
        assert_eq!(modules.len(), MODULES.len());
        for (name, source) in &modules {
            assert!(!source.starts_with("throw new Error"), "{name}: {source}");
        }
    }
}
