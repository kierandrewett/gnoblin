// Run against the freshly built Mutter typelib in a Gnoblin session mode.
import GLib from "gi://GLib";
import Meta from "gi://Meta";
import {
    applyCompositorPreferences,
    applyWindowPreferences,
    COMPOSITOR_PREFERENCES,
    WINDOW_PREFERENCES,
} from
    "../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js";

if (GLib.getenv("GNOME_SHELL_SESSION_MODE") !== "gnoblin")
    throw new Error("Native preference test requires Gnoblin session mode");

function assert(condition, message) {
    if (!condition) throw new Error(message);
}

applyWindowPreferences({
    ...WINDOW_PREFERENCES,
    "focus-mode": "sloppy",
    "auto-raise": true,
    "action-middle-click-titlebar": "minimize",
    "dynamic-workspaces": true,
    "edge-tiling": true,
    "workspace-names": ["Main", "Chat"],
});
assert(Meta.prefs_get_focus_mode() === 1, "native focus mode follows Lua");
assert(Meta.prefs_get_auto_raise(), "native auto-raise follows Lua");
assert(Meta.prefs_get_action_middle_click_titlebar() === 4, "native titlebar action follows Lua");
assert(Meta.prefs_get_dynamic_workspaces(), "native workspace mode follows Lua");
assert(Meta.prefs_get_edge_tiling(), "native edge tiling follows Lua");
assert(Meta.prefs_get_workspace_name(0) === "Main" && Meta.prefs_get_workspace_name(1) === "Chat",
    `native workspace names follow Lua (got ${Meta.prefs_get_workspace_name(0)} / ${Meta.prefs_get_workspace_name(1)})`);

applyCompositorPreferences({
    ...COMPOSITOR_PREFERENCES,
    "enable-animations": false,
    "visual-bell": true,
    "audible-bell": false,
    "visual-bell-type": "frame-flash",
});
assert(!Meta.prefs_get_gnome_animations(), "native animation preference follows Lua");
assert(Meta.prefs_get_visual_bell() && !Meta.prefs_bell_is_audible(),
    "native bell preferences follow Lua");

applyWindowPreferences(WINDOW_PREFERENCES);
assert(Meta.prefs_get_focus_mode() === 0 && !Meta.prefs_get_auto_raise() &&
    Meta.prefs_get_action_middle_click_titlebar() === 6 &&
    !Meta.prefs_get_dynamic_workspaces() && !Meta.prefs_get_edge_tiling(),
    "removing Lua values restores Gnoblin defaults");
applyCompositorPreferences(COMPOSITOR_PREFERENCES);
assert(Meta.prefs_get_gnome_animations() && !Meta.prefs_get_visual_bell() && Meta.prefs_bell_is_audible(),
    "removing Lua values restores Gnoblin compositor defaults");
print("PASS: native window preferences apply and reset without GSettings");
