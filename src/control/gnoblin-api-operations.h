#ifndef GNOBLIN_API_OPERATIONS_H
#define GNOBLIN_API_OPERATIONS_H

/* Every wire operation of the native control API, spelled once.
 *
 * Socket clients, the Lua runtime, the compositor and gnoblinctl name an operation by the same
 * string: "window.close", "workspace.rename", "override.revoke". Each of them used to keep its own
 * chain of string comparisons for it. This table holds what those chains shared: the name, the
 * first API minor that offers the operation and the error that older clients get, the minor that
 * gnoblinctl asks for, who may send it, which packet fields the supervisor adds and which
 * compositor routine runs it. The lua_api[] table of the Lua runtime points here for the
 * operation behind each function (gnoblin-lua.c), so the name of an operation is not repeated
 * there either.
 *
 * The header holds data and inline lookups only. libmutter, the gnoblin executable and gnoblinctl
 * each include it; none of them has to link the others. Code that does something specific for one
 * operation keeps that code, and tests the operation by its id, as in
 * `op->id == GNOBLIN_OP_WINDOW_FOCUS`, not by its name.
 *
 * A row whose name ends in "." with GN_OP_FAMILY stands for every name that starts with it. An
 * exact row wins over a family row. A row with no gate or no gnoblinctl minor takes it from its
 * family. */
#include <glib.h>
#include <string.h>

/* Flags of a row. */
enum {
    /* Listed in the "methods" of the hello event, in table order. */
    GN_OP_HELLO = 1 << 0,
    /* The row names a family of operations by prefix. */
    GN_OP_FAMILY = 1 << 1,
    /* Runs inside a shortcut, pointer or window-menu gesture: the supervisor attaches the
     * focus context that the gesture left behind. */
    GN_OP_GRANT = 1 << 2,
    /* The supervisor attaches the runtime generation, so the compositor can refuse an operation
     * that a replaced runtime queued. */
    GN_OP_GENERATION = 1 << 3,
    /* The compositor reads the ID of the socket client that made the call. */
    GN_OP_CLIENT_ID = 1 << 4,
    /* A Lua workspace action: lua_workspace_action builds its arguments. */
    GN_OP_WORKSPACE = 1 << 5,
    /* A call from Lua that is not a record method takes no arguments. */
    GN_OP_NOTHING = 1 << 6,
    /* A snapshot read that the runtime answers (native_api_read_method). */
    GN_OP_READ = 1 << 8,
    /* An animation read that the compositor answers (native_api_animation_read_method). */
    GN_OP_ANIM_READ = 1 << 9,
    /* gnoblinctl sends the API version with the request. */
    GN_OP_VERSIONED = 1 << 10,
    /* gnoblinctl waits for the operation to complete, not only for the reply. */
    GN_OP_WAITS = 1 << 11,
    /* The compositor refuses the call from a replaced runtime, a suspended worker and, except
     * for process.kill, a locked session. */
    GN_OP_PROCESS_GATE = 1 << 12,
    /* The compositor refuses the call from a replaced runtime. */
    GN_OP_SNAP_GATE = 1 << 13,
    /* The gate of 1.19 for snapshot reads is checked before the gate of the row. */
    GN_OP_READS_FIRST = 1 << 14,
    /* The Lua result is the Workspace that the operation made, renamed or activated. */
    GN_OP_RESULT_WORKSPACE = 1 << 15,
    /* The Lua result is the live object that the operation started or changed. */
    GN_OP_RESULT_LIVE = 1 << 16,
};

/* The compositor routine that runs an operation. GN_DISPATCH_NATIVE is the typed Mutter API
 * (meta_gnoblin_dispatch_native_api); a name that has no row goes there too. */
typedef enum {
    GN_DISPATCH_NATIVE = 0,
    GN_DISPATCH_AUTH,
    GN_DISPATCH_CONSOLE_TOGGLE,
    GN_DISPATCH_GRAB,
    GN_DISPATCH_GRANT,
    GN_DISPATCH_IDLE,
    GN_DISPATCH_INPUT_SELECT,
    GN_DISPATCH_INSERT_TEXT,
    GN_DISPATCH_LAUNCH,
    GN_DISPATCH_LOCATION,
    GN_DISPATCH_MONITOR_PRIVACY,
    GN_DISPATCH_ORIENTATION_LOCK,
    GN_DISPATCH_OVERRIDE,
    GN_DISPATCH_POWER,
    GN_DISPATCH_PRESENT,
    GN_DISPATCH_PRIVACY_STOP,
    GN_DISPATCH_PROCESS_KILL,
    GN_DISPATCH_PROCESS_SPAWN,
    GN_DISPATCH_PROMPT,
    GN_DISPATCH_RESTORE_DEFAULT,
    GN_DISPATCH_RESTORE_OR_MINIMIZE,
    GN_DISPATCH_SESSION_LOCK,
    GN_DISPATCH_SESSION_LOGOUT,
    GN_DISPATCH_SET_MAXIMIZED,
    GN_DISPATCH_SET_VISUAL,
    GN_DISPATCH_SHORTCUT,
    GN_DISPATCH_SHORTCUT_ARM,
    GN_DISPATCH_SHORTCUT_CAPTURE,
    GN_DISPATCH_SNAP_COMMIT,
    GN_DISPATCH_SNAP_OFFER,
    GN_DISPATCH_THUMBNAIL,
    GN_DISPATCH_TWEEN_CONTROL,
    GN_DISPATCH_TWEEN_PLAY,
} GnoblinApiDispatch;

/* Shared gate messages: the text a client gets when it asks for the operation below the minor that
 * offers it. A row with no message says "<name> requires API version 1.<minor>". */
#define MSG_READS "shared snapshot reads require API version 1.19"
#define MSG_COLLECTIONS "Lua snapshot collection reads require API version 1.37"
#define MSG_INPUT_SOURCES "input source methods require API version 1.6"
#define MSG_ORIENTATION "orientation lock methods require API version 1.66"
#define MSG_PRIVACY_SCREEN "monitor privacy-screen methods require API version 1.74"
#define MSG_INTERACTIVE "interactive window operations require API version 1.12"
#define MSG_DYNAMIC_SHORTCUTS "dynamic shortcut methods require API version 1.11"
#define MSG_POWER "session power methods require API version 1.32"
#define MSG_PRIVACY_STOP "privacy stop methods require API version 1.31"
#define MSG_OUTPUTS "output reads require API version 1.82"
#define MSG_TEXT_SNAP "text insertion and keyboard snap methods require API version 1.28"
#define MSG_ANIMATION "animation methods require API version 1.18"
#define MSG_AUTH "auth methods require API version 1.78"
#define MSG_PROMPT "prompt methods require API version 1.79"
#define MSG_LAUNCH "launch methods require API version 1.7"

/* X(id, name, gate, gate message, gnoblinctl minor, flags, dispatch).
 * The rows that carry GN_OP_HELLO come first, in the order of the hello event. */
/* clang-format off */
#define GNOBLIN_API_OPERATIONS(X) \
    X(CAPABILITIES_LIST, "capabilities.list", 19, MSG_READS, 19, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(FOCUS_HISTORY, "focus.history", 19, MSG_READS, 19, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(FOCUS_POLICY, "focus.policy", 19, MSG_READS, 19, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(CONFIG_PATH, "config.path", 19, MSG_READS, 0, GN_OP_HELLO|GN_OP_READ, GN_DISPATCH_NATIVE) \
    X(CONFIG_DEFAULT, "config.default", 19, MSG_READS, 0, GN_OP_HELLO|GN_OP_READ, GN_DISPATCH_NATIVE) \
    X(INPUT_DEVICES, "input.devices", 3, NULL, 46, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(INPUT_SOURCES, "input.sources", 6, MSG_INPUT_SOURCES, 46, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(INPUT_CURRENT_SOURCE, "input.current_source", 6, MSG_INPUT_SOURCES, 46, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(INPUT_ORIENTATION_LOCK, "input.orientation_lock", 66, MSG_ORIENTATION, 66, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(INPUT_SELECT, "input.select", 6, MSG_INPUT_SOURCES, 6, GN_OP_HELLO|GN_OP_VERSIONED|GN_OP_WAITS|GN_OP_RESULT_LIVE, GN_DISPATCH_INPUT_SELECT) \
    X(INPUT_SET_ORIENTATION_LOCK, "input.set_orientation_lock", 66, MSG_ORIENTATION, 66, GN_OP_HELLO|GN_OP_VERSIONED|GN_OP_WAITS, GN_DISPATCH_ORIENTATION_LOCK) \
    X(INPUT_TEXT_TARGET, "input.text_target", 28, MSG_TEXT_SNAP, 28, GN_OP_HELLO|GN_OP_GRANT, GN_DISPATCH_GRANT) \
    X(INPUT_INSERT_TEXT, "input.insert_text", 28, MSG_TEXT_SNAP, 28, GN_OP_HELLO, GN_DISPATCH_INSERT_TEXT) \
    X(LAUNCH_STATUS, "launch.status", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_LAUNCH) \
    X(LAUNCH_BEGIN, "launch.begin", 0, NULL, 50, GN_OP_HELLO|GN_OP_VERSIONED|GN_OP_RESULT_LIVE, GN_DISPATCH_LAUNCH) \
    X(LAUNCH_END, "launch.end", 0, NULL, 50, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_LAUNCH) \
    X(LAUNCHES_LIST, "launches.list", 37, MSG_COLLECTIONS, 0, GN_OP_HELLO|GN_OP_READ, GN_DISPATCH_NATIVE) \
    X(LAUNCHES_SNAPSHOT, "launches.snapshot", 39, NULL, 39, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(SHORTCUTS_LIST, "shortcuts.list", 40, NULL, 40, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(SHORTCUTS_ACTIONS, "shortcuts.actions", 41, NULL, 41, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(LAYER_LIST, "layer.list", 2, NULL, 2, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(LAYERS_LIST, "layers.list", 37, MSG_COLLECTIONS, 37, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(MONITOR_LIST, "monitor.list", 0, NULL, 0, GN_OP_HELLO, GN_DISPATCH_NATIVE) \
    X(MONITORS_LIST, "monitors.list", 37, MSG_COLLECTIONS, 37, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(OUTPUT_LIST, "output.list", 19, MSG_READS, 0, GN_OP_HELLO|GN_OP_READ, GN_DISPATCH_NATIVE) \
    X(OUTPUTS_LIST, "outputs.list", 82, MSG_OUTPUTS, 82, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(OUTPUTS_RENDERER, "outputs.renderer", 82, MSG_OUTPUTS, 82, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(MONITORS_PRIVACY_SCREEN, "monitors.privacy_screen", 74, MSG_PRIVACY_SCREEN, 74, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(MONITORS_SET_PRIVACY_SCREEN, "monitors.set_privacy_screen", 74, MSG_PRIVACY_SCREEN, 74, GN_OP_HELLO|GN_OP_VERSIONED|GN_OP_WAITS, GN_DISPATCH_MONITOR_PRIVACY) \
    X(SETTINGS, "settings", 19, MSG_READS, 19, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(SHORTCUT_ACTIONS, "shortcut.actions", 5, NULL, 5, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(SHORTCUT_BIND, "shortcut.bind", 11, MSG_DYNAMIC_SHORTCUTS, 22, GN_OP_HELLO|GN_OP_RESULT_LIVE, GN_DISPATCH_SHORTCUT) \
    X(SHORTCUT_CAPTURE, "shortcut.capture", 8, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED|GN_OP_WAITS, GN_DISPATCH_SHORTCUT_CAPTURE) \
    X(SHORTCUT_LIST, "shortcut.list", 9, NULL, 9, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(SHORTCUT_SESSION_END, "shortcut.session.end", 64, NULL, 64, GN_OP_HELLO, GN_DISPATCH_SHORTCUT) \
    X(SHORTCUT_UNBIND, "shortcut.unbind", 11, MSG_DYNAMIC_SHORTCUTS, 11, GN_OP_HELLO, GN_DISPATCH_SHORTCUT) \
    X(PRIVACY_STATE, "privacy.state", 17, NULL, 47, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(PRIVACY_STOP_SHARING, "privacy.stop_sharing", 31, MSG_PRIVACY_STOP, 31, GN_OP_HELLO|GN_OP_NOTHING|GN_OP_VERSIONED, GN_DISPATCH_PRIVACY_STOP) \
    X(PRIVACY_STOP_RECORDING, "privacy.stop_recording", 31, MSG_PRIVACY_STOP, 31, GN_OP_HELLO|GN_OP_NOTHING|GN_OP_VERSIONED, GN_DISPATCH_PRIVACY_STOP) \
    X(LOCATION_AUTHORIZE_APP, "location.authorize_app", 65, NULL, 65, GN_OP_HELLO|GN_OP_CLIENT_ID, GN_DISPATCH_LOCATION) \
    X(AUTH_BEGIN, "auth.begin", 0, NULL, 78, GN_OP_HELLO|GN_OP_CLIENT_ID, GN_DISPATCH_AUTH) \
    X(AUTH_RESPOND, "auth.respond", 0, NULL, 78, GN_OP_HELLO|GN_OP_CLIENT_ID, GN_DISPATCH_AUTH) \
    X(AUTH_CANCEL, "auth.cancel", 0, NULL, 78, GN_OP_HELLO|GN_OP_CLIENT_ID, GN_DISPATCH_AUTH) \
    X(PROMPT_RESPOND, "prompt.respond", 0, NULL, 79, GN_OP_HELLO|GN_OP_CLIENT_ID, GN_DISPATCH_PROMPT) \
    X(PROMPT_CANCEL, "prompt.cancel", 0, NULL, 79, GN_OP_HELLO|GN_OP_CLIENT_ID, GN_DISPATCH_PROMPT) \
    X(ANIMATION_LIST, "animation.list", 0, NULL, 0, GN_OP_HELLO|GN_OP_ANIM_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(ANIMATION_GET, "animation.get", 0, NULL, 0, GN_OP_HELLO|GN_OP_ANIM_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(ANIMATION_SURFACES, "animation.surfaces", 0, NULL, 0, GN_OP_HELLO|GN_OP_ANIM_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(ANIMATION_INSPECT, "animation.inspect", 0, NULL, 0, GN_OP_HELLO|GN_OP_ANIM_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(ANIMATION_PREVIEW, "animation.preview", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED|GN_OP_WAITS, GN_DISPATCH_NATIVE) \
    X(ANIMATION_SEEK, "animation.seek", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(ANIMATION_STEP, "animation.step", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(ANIMATION_PLAY, "animation.play", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(ANIMATION_PAUSE, "animation.pause", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(ANIMATION_STOP, "animation.stop", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_LIST, "window.list", 19, MSG_READS, 0, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOWS_LIST, "windows.list", 37, MSG_COLLECTIONS, 37, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_MATCH, "window.match", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_THUMBNAIL, "window.thumbnail", 23, NULL, 23, GN_OP_HELLO|GN_OP_CLIENT_ID|GN_OP_VERSIONED|GN_OP_WAITS, GN_DISPATCH_THUMBNAIL) \
    X(WINDOW_SNAP_OFFER, "window.snap.offer", 26, NULL, 0, GN_OP_HELLO|GN_OP_GENERATION|GN_OP_VERSIONED, GN_DISPATCH_SNAP_OFFER) \
    X(WINDOW_ACTION, "window.action", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_CLOSE, "window.close", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_KILL, "window.kill", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_RAISE, "window.raise", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_LOWER, "window.lower", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_MINIMIZE, "window.minimize", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_TOGGLE_MINIMIZE, "window.toggle_minimize", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_UNMINIMIZE, "window.unminimize", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_RESTORE, "window.restore", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_RESTORE_OR_MINIMIZE, "window.restore_or_minimize", 38, NULL, 48, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_RESTORE_OR_MINIMIZE) \
    X(WINDOW_SET_MAXIMIZED, "window.set_maximized", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_SET_MAXIMIZED) \
    X(WINDOW_SET_FULLSCREEN, "window.set_fullscreen", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_SET_ABOVE, "window.set_above", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_SET_STICKY, "window.set_sticky", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_MOVE, "window.move", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_RESIZE, "window.resize", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_MOVE_TO_WORKSPACE, "window.move_to_workspace", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_MOVE_TO_MONITOR, "window.move_to_monitor", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_FOCUS, "window.focus", 10, NULL, 0, GN_OP_HELLO|GN_OP_GRANT|GN_OP_VERSIONED, GN_DISPATCH_GRANT) \
    X(WINDOW_SNAP_CONTEXT, "window.snap_context", 28, MSG_TEXT_SNAP, 0, GN_OP_HELLO|GN_OP_GRANT|GN_OP_GENERATION|GN_OP_VERSIONED|GN_OP_SNAP_GATE, GN_DISPATCH_GRANT) \
    X(WINDOW_SNAP, "window.snap", 28, MSG_TEXT_SNAP, 0, GN_OP_HELLO|GN_OP_GENERATION|GN_OP_VERSIONED|GN_OP_SNAP_GATE, GN_DISPATCH_SNAP_COMMIT) \
    X(WINDOW_BEGIN_MOVE, "window.begin_move", 12, MSG_INTERACTIVE, 0, GN_OP_HELLO|GN_OP_GRANT|GN_OP_VERSIONED, GN_DISPATCH_GRANT) \
    X(WINDOW_BEGIN_RESIZE, "window.begin_resize", 12, MSG_INTERACTIVE, 0, GN_OP_HELLO|GN_OP_GRANT|GN_OP_VERSIONED, GN_DISPATCH_GRANT) \
    X(WORKSPACE_LIST, "workspace.list", 0, NULL, 0, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WORKSPACES_LIST, "workspaces.list", 37, MSG_COLLECTIONS, 37, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WORKSPACE_CREATE, "workspace.create", 0, NULL, 0, GN_OP_HELLO|GN_OP_WORKSPACE|GN_OP_VERSIONED|GN_OP_RESULT_WORKSPACE, GN_DISPATCH_NATIVE) \
    X(WORKSPACE_RENAME, "workspace.rename", 0, NULL, 0, GN_OP_HELLO|GN_OP_WORKSPACE|GN_OP_VERSIONED|GN_OP_RESULT_WORKSPACE, GN_DISPATCH_NATIVE) \
    X(WORKSPACE_REMOVE, "workspace.remove", 0, NULL, 0, GN_OP_HELLO|GN_OP_WORKSPACE|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WORKSPACE_SWITCH, "workspace.switch", 0, NULL, 0, GN_OP_HELLO|GN_OP_WORKSPACE|GN_OP_VERSIONED|GN_OP_RESULT_WORKSPACE, GN_DISPATCH_NATIVE) \
    X(WORKSPACE_NEXT, "workspace.next", 0, NULL, 0, GN_OP_HELLO|GN_OP_WORKSPACE|GN_OP_NOTHING|GN_OP_VERSIONED|GN_OP_RESULT_WORKSPACE, GN_DISPATCH_NATIVE) \
    X(WORKSPACE_PREVIOUS, "workspace.previous", 0, NULL, 0, GN_OP_HELLO|GN_OP_WORKSPACE|GN_OP_NOTHING|GN_OP_VERSIONED|GN_OP_RESULT_WORKSPACE, GN_DISPATCH_NATIVE) \
    X(WORKSPACE_MOVE_ACTIVE, "workspace.move_active", 0, NULL, 0, GN_OP_HELLO|GN_OP_WORKSPACE|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WORKSPACE_MOVE_WINDOW, "workspace.move_window", 0, NULL, 0, GN_OP_HELLO|GN_OP_WORKSPACE|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(RUNTIME_RELOAD_CONFIG, "runtime.reload_config", 20, NULL, 20, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(CONFIG_RESTORE_DEFAULT, "config.restore_default", 32, NULL, 32, GN_OP_HELLO|GN_OP_VERSIONED, GN_DISPATCH_RESTORE_DEFAULT) \
    X(RUNTIME_STATUS, "runtime.status", 67, NULL, 67, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED|GN_OP_READS_FIRST, GN_DISPATCH_NATIVE) \
    X(SESSION_LOCK, "session.lock", 21, NULL, 21, GN_OP_HELLO|GN_OP_NOTHING|GN_OP_VERSIONED, GN_DISPATCH_SESSION_LOCK) \
    X(SESSION_ACTIVITY, "session.activity", 24, NULL, 24, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(SESSION_LOGOUT, "session.logout", 32, NULL, 32, GN_OP_HELLO|GN_OP_NOTHING|GN_OP_VERSIONED, GN_DISPATCH_SESSION_LOGOUT) \
    X(SESSION_SUSPEND, "session.suspend", 32, MSG_POWER, 32, GN_OP_HELLO|GN_OP_NOTHING|GN_OP_VERSIONED, GN_DISPATCH_POWER) \
    X(SESSION_HIBERNATE, "session.hibernate", 32, MSG_POWER, 32, GN_OP_HELLO|GN_OP_NOTHING|GN_OP_VERSIONED, GN_DISPATCH_POWER) \
    X(SESSION_REBOOT, "session.reboot", 32, MSG_POWER, 32, GN_OP_HELLO|GN_OP_NOTHING|GN_OP_VERSIONED, GN_DISPATCH_POWER) \
    X(SESSION_POWEROFF, "session.poweroff", 32, MSG_POWER, 32, GN_OP_HELLO|GN_OP_NOTHING|GN_OP_VERSIONED, GN_DISPATCH_POWER) \
    X(SESSION_STATUS, "session.status", 29, NULL, 29, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(APPEARANCE_COLOR_SCHEME, "appearance.color_scheme", 70, NULL, 70, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(LAYER_ANIMATION_POLICY, "layer.animation_policy", 31, NULL, 31, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(VERSION, "version", 19, MSG_READS, 19, GN_OP_HELLO|GN_OP_READ|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(CONSOLE_TOGGLE, "console.toggle", 0, NULL, 0, GN_OP_NOTHING, GN_DISPATCH_CONSOLE_TOGGLE) \
    X(IDLE_WATCH, "idle.watch", 0, NULL, 0, GN_OP_GENERATION, GN_DISPATCH_IDLE) \
    X(IDLE_CANCEL, "idle.cancel", 0, NULL, 0, GN_OP_GENERATION, GN_DISPATCH_IDLE) \
    X(INPUT_GRAB, "input.grab", 0, NULL, 0, GN_OP_GENERATION, GN_DISPATCH_GRAB) \
    X(INPUT_GRAB_RENEW, "input.grab.renew", 0, NULL, 0, GN_OP_GENERATION, GN_DISPATCH_GRAB) \
    X(INPUT_GRAB_RELEASE, "input.grab.release", 0, NULL, 0, GN_OP_GENERATION, GN_DISPATCH_GRAB) \
    X(OVERRIDE_REVOKE, "override.revoke", 0, NULL, 0, GN_OP_GENERATION, GN_DISPATCH_OVERRIDE) \
    X(SETTINGS_OVERRIDE, "settings.override", 0, NULL, 0, GN_OP_GENERATION, GN_DISPATCH_OVERRIDE) \
    X(WINDOW_PRESENT, "window.present", 0, NULL, 0, GN_OP_VERSIONED, GN_DISPATCH_PRESENT) \
    X(WINDOW_SET_EFFECT, "window.set_effect", 0, NULL, 0, GN_OP_GENERATION|GN_OP_VERSIONED, GN_DISPATCH_OVERRIDE) \
    X(WINDOW_SET_MINIMIZED, "window.set_minimized", 0, NULL, 0, GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WINDOW_SET_VISUAL, "window.set_visual", 0, NULL, 0, GN_OP_GENERATION|GN_OP_VERSIONED, GN_DISPATCH_SET_VISUAL) \
    X(PROCESS_SPAWN, "process.spawn", 0, NULL, 0, GN_OP_GENERATION|GN_OP_PROCESS_GATE, GN_DISPATCH_PROCESS_SPAWN) \
    X(PROCESS_KILL, "process.kill", 0, NULL, 0, GN_OP_GENERATION|GN_OP_PROCESS_GATE, GN_DISPATCH_PROCESS_KILL) \
    X(TWEEN_PLAY, "tween.play", 0, NULL, 0, GN_OP_GENERATION, GN_DISPATCH_TWEEN_PLAY) \
    X(TWEEN_PAUSE, "tween.pause", 0, NULL, 0, GN_OP_GENERATION, GN_DISPATCH_TWEEN_CONTROL) \
    X(TWEEN_RESUME, "tween.resume", 0, NULL, 0, GN_OP_GENERATION, GN_DISPATCH_TWEEN_CONTROL) \
    X(TWEEN_SEEK, "tween.seek", 0, NULL, 0, GN_OP_GENERATION, GN_DISPATCH_TWEEN_CONTROL) \
    X(TWEEN_KILL, "tween.kill", 0, NULL, 0, GN_OP_GENERATION, GN_DISPATCH_TWEEN_CONTROL) \
    X(SHORTCUT_SESSION_ARM, "shortcut.session.arm", 0, NULL, 0, 0, GN_DISPATCH_SHORTCUT_ARM) \
    X(INPUT_SELECT_SOURCE, "input.select_source", 6, MSG_INPUT_SOURCES, 0, 0, GN_DISPATCH_INPUT_SELECT) \
    X(WINDOW_FAMILY, "window.", 0, NULL, 64, GN_OP_FAMILY|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(WORKSPACE_FAMILY, "workspace.", 0, NULL, 64, GN_OP_FAMILY|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(ANIMATION_FAMILY, "animation.", 18, MSG_ANIMATION, 18, GN_OP_FAMILY|GN_OP_VERSIONED, GN_DISPATCH_NATIVE) \
    X(AUTH_FAMILY, "auth.", 78, MSG_AUTH, 0, GN_OP_FAMILY|GN_OP_CLIENT_ID, GN_DISPATCH_AUTH) \
    X(PROMPT_FAMILY, "prompt.", 79, MSG_PROMPT, 0, GN_OP_FAMILY|GN_OP_CLIENT_ID, GN_DISPATCH_PROMPT) \
    X(LAUNCH_FAMILY, "launch.", 7, MSG_LAUNCH, 0, GN_OP_FAMILY, GN_DISPATCH_LAUNCH) \
    X(PROCESS_FAMILY, "process.", 0, NULL, 0, GN_OP_FAMILY|GN_OP_GENERATION|GN_OP_PROCESS_GATE, GN_DISPATCH_NATIVE) \
    X(TWEEN_FAMILY, "tween.", 0, NULL, 0, GN_OP_FAMILY|GN_OP_GENERATION, GN_DISPATCH_TWEEN_CONTROL)
/* clang-format on */

typedef enum {
#define X(id, name, gate, message, cli_minor, flags, dispatch) GNOBLIN_OP_##id,
    GNOBLIN_API_OPERATIONS(X)
#undef X
        GNOBLIN_OP_COUNT
} GnoblinApiOp;

typedef struct {
    GnoblinApiOp id;
    const char* name;
    guint8 gate;
    const char* gate_message;
    guint8 cli_minor;
    guint32 flags;
    GnoblinApiDispatch dispatch;
} GnoblinApiOperation;

static const GnoblinApiOperation gnoblin_api_operations[] = {
#define X(id, name, gate, message, cli_minor, flags, dispatch)                                     \
    {GNOBLIN_OP_##id, name, gate, message, cli_minor, flags, dispatch},
    GNOBLIN_API_OPERATIONS(X)
#undef X
};

/* The row of an operation: the exact name, else the family that the name belongs to. NULL when the
 * name has neither. */
static inline const GnoblinApiOperation* gnoblin_api_operation_find(const char* name) {
    const GnoblinApiOperation* family = NULL;
    if (!name)
        return NULL;
    for (gsize i = 0; i < G_N_ELEMENTS(gnoblin_api_operations); i++) {
        const GnoblinApiOperation* row = &gnoblin_api_operations[i];
        if (row->flags & GN_OP_FAMILY) {
            if (!family && g_str_has_prefix(name, row->name))
                family = row;
        } else if (g_str_equal(row->name, name)) {
            return row;
        }
    }
    return family;
}

/* The family row that a name belongs to, or NULL. */
static inline const GnoblinApiOperation* gnoblin_api_operation_family(const char* name) {
    for (gsize i = 0; i < G_N_ELEMENTS(gnoblin_api_operations); i++)
        if ((gnoblin_api_operations[i].flags & GN_OP_FAMILY) &&
            g_str_has_prefix(name, gnoblin_api_operations[i].name))
            return &gnoblin_api_operations[i];
    return NULL;
}

/* The wire name of an operation id. */
static inline const char* gnoblin_api_operation_name(GnoblinApiOp id) {
    return gnoblin_api_operations[id].name;
}

/* The error for a client of API minor `client_minor` that asks for `method`, or NULL when its
 * minor is high enough or the method has no gate. The caller frees the text. */
static inline char* gnoblin_api_operation_gate_error(const char* method, guint client_minor) {
    const GnoblinApiOperation* row = gnoblin_api_operation_find(method);
    const GnoblinApiOperation* source = row;
    if (row && (row->flags & GN_OP_READS_FIRST) && client_minor < 19)
        return g_strdup(MSG_READS);
    if (row && !row->gate) {
        const GnoblinApiOperation* family = gnoblin_api_operation_family(method);
        if (family && family != row && family->gate)
            source = family;
    }
    if (!source || !source->gate || client_minor >= source->gate)
        return NULL;
    if (source->gate_message)
        return g_strdup(source->gate_message);
    return g_strdup_printf("%s requires API version 1.%u", source->name, (guint)source->gate);
}

/* The API minor that gnoblinctl asks for when it sends `method`. */
static inline guint gnoblin_api_operation_cli_minor(const char* method) {
    const GnoblinApiOperation* row = gnoblin_api_operation_find(method);
    if (row && row->cli_minor)
        return row->cli_minor;
    const GnoblinApiOperation* family = gnoblin_api_operation_family(method);
    return family && family->cli_minor ? family->cli_minor : 8;
}

/* Whether `method` starts with the prefix of the family row `family_id`. */
static inline gboolean gnoblin_api_operation_in_family(const char* method, GnoblinApiOp family_id) {
    return g_str_has_prefix(method, gnoblin_api_operations[family_id].name);
}

/* Whether `method` is the operation `id`. */
static inline gboolean gnoblin_api_operation_is(const char* method, GnoblinApiOp id) {
    const GnoblinApiOperation* row = gnoblin_api_operation_find(method);
    return row && row->id == id;
}

/* Whether `method` has a row with all of `flags`. */
static inline gboolean gnoblin_api_operation_has(const char* method, guint32 flags) {
    const GnoblinApiOperation* row = gnoblin_api_operation_find(method);
    return row && (row->flags & flags) == flags;
}
#endif
