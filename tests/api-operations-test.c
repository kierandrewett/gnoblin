#include "gnoblin-lua-api.h"

/* The wire contract that the operation table in gnoblin-api-operations.h keeps for socket
 * clients: the hello list, the version gates and their errors, the minors that gnoblinctl asks
 * for, and that every function of lua_api[] points at a row. The names are spelled out here on
 * purpose: the test pins what clients see, while the table is the one place the code spells them.
 */

static const char* const hello_methods[] = {
    "capabilities.list",
    "focus.history",
    "focus.policy",
    "config.path",
    "config.default",
    "input.devices",
    "input.sources",
    "input.current_source",
    "input.orientation_lock",
    "input.select",
    "input.set_orientation_lock",
    "input.text_target",
    "input.insert_text",
    "launch.status",
    "launch.begin",
    "launch.end",
    "launches.list",
    "launches.snapshot",
    "shortcuts.list",
    "shortcuts.actions",
    "layer.list",
    "layers.list",
    "monitor.list",
    "monitors.list",
    "output.list",
    "outputs.list",
    "outputs.renderer",
    "monitors.privacy_screen",
    "monitors.set_privacy_screen",
    "settings",
    "shortcut.actions",
    "shortcut.bind",
    "shortcut.capture",
    "shortcut.list",
    "shortcut.session.end",
    "shortcut.unbind",
    "privacy.state",
    "privacy.stop_sharing",
    "privacy.stop_recording",
    "location.authorize_app",
    "auth.begin",
    "auth.respond",
    "auth.cancel",
    "prompt.respond",
    "prompt.cancel",
    "animation.list",
    "animation.get",
    "animation.surfaces",
    "animation.inspect",
    "animation.preview",
    "animation.seek",
    "animation.step",
    "animation.play",
    "animation.pause",
    "animation.stop",
    "window.list",
    "windows.list",
    "window.match",
    "window.thumbnail",
    "window.snap.offer",
    "window.action",
    "window.close",
    "window.kill",
    "window.raise",
    "window.lower",
    "window.minimize",
    "window.toggle_minimize",
    "window.unminimize",
    "window.restore",
    "window.restore_or_minimize",
    "window.set_maximized",
    "window.set_fullscreen",
    "window.set_above",
    "window.set_sticky",
    "window.move",
    "window.resize",
    "window.move_to_workspace",
    "window.move_to_monitor",
    "window.focus",
    "window.snap_context",
    "window.snap",
    "window.begin_move",
    "window.begin_resize",
    "workspace.list",
    "workspaces.list",
    "workspace.create",
    "workspace.rename",
    "workspace.remove",
    "workspace.switch",
    "workspace.next",
    "workspace.previous",
    "workspace.move_active",
    "workspace.move_window",
    "runtime.reload_config",
    "config.restore_default",
    "runtime.status",
    "session.lock",
    "session.activity",
    "session.logout",
    "session.suspend",
    "session.hibernate",
    "session.reboot",
    "session.poweroff",
    "session.status",
    "appearance.color_scheme",
    "layer.animation_policy",
    "version",
    NULL,
};

static void test_hello_list(void) {
    gsize listed = 0;
    for (gsize i = 0; i < G_N_ELEMENTS(gnoblin_api_operations); i++) {
        if (!(gnoblin_api_operations[i].flags & GN_OP_HELLO))
            continue;
        g_assert_nonnull(hello_methods[listed]);
        g_assert_cmpstr(gnoblin_api_operations[i].name, ==, hello_methods[listed]);
        listed++;
    }
    g_assert_null(hello_methods[listed]);
}

static void test_rows_are_consistent(void) {
    for (gsize i = 0; i < G_N_ELEMENTS(gnoblin_api_operations); i++) {
        const GnoblinApiOperation* row = &gnoblin_api_operations[i];
        g_assert_cmpuint(row->id, ==, i);
        g_assert_true(!!(row->flags & GN_OP_FAMILY) == g_str_has_suffix(row->name, "."));
        if (!(row->flags & GN_OP_FAMILY))
            g_assert_true(gnoblin_api_operation_find(row->name) == row);
        for (gsize j = 0; j < i; j++)
            g_assert_cmpstr(gnoblin_api_operations[j].name, !=, row->name);
        if (row->gate_message)
            g_assert_cmpuint(row->gate, >, 0);
    }
}

static void assert_gate(const char* method, guint minor, const char* expected) {
    g_autofree char* error = gnoblin_api_operation_gate_error(method, minor);
    g_assert_cmpstr(error, ==, expected);
}

static void test_gate_errors(void) {
    assert_gate("layer.list", 1, "layer.list requires API version 1.2");
    assert_gate("layer.list", 2, NULL);
    assert_gate("input.select_source", 5, "input source methods require API version 1.6");
    assert_gate("shortcut.unbind", 10, "dynamic shortcut methods require API version 1.11");
    assert_gate("session.reboot", 31, "session power methods require API version 1.32");
    assert_gate("session.logout", 31, "session.logout requires API version 1.32");
    assert_gate("windows.list", 36, "Lua snapshot collection reads require API version 1.37");
    assert_gate("windows.list", 37, NULL);
    assert_gate("window.list", 18, "shared snapshot reads require API version 1.19");
    assert_gate("window.list", 19, NULL);
    assert_gate("outputs.list", 81, "output reads require API version 1.82");
    assert_gate("session.status", 28, "session.status requires API version 1.29");
    assert_gate("monitors.privacy_screen", 73,
                "monitor privacy-screen methods require API version 1.74");
    /* runtime.status is a snapshot read from 1.19 and an operation of its own from 1.67. */
    assert_gate("runtime.status", 18, "shared snapshot reads require API version 1.19");
    assert_gate("runtime.status", 19, "runtime.status requires API version 1.67");
    assert_gate("runtime.status", 67, NULL);
    /* A family gates every name under its prefix, listed or not. */
    assert_gate("auth.begin", 77, "auth methods require API version 1.78");
    assert_gate("auth.unknown", 77, "auth methods require API version 1.78");
    assert_gate("prompt.cancel", 78, "prompt methods require API version 1.79");
    assert_gate("animation.list", 17, "animation methods require API version 1.18");
    assert_gate("animation.unknown", 17, "animation methods require API version 1.18");
    assert_gate("launch.begin", 6, "launch methods require API version 1.7");
    assert_gate("launches.snapshot", 38, "launches.snapshot requires API version 1.39");
    /* Operations with no gate, and names that are not operations. */
    assert_gate("window.close", 1, NULL);
    assert_gate("workspace.rename", 1, NULL);
    assert_gate("bogus.method", 1, NULL);
    assert_gate("", 1, NULL);
}

static void test_cli_minor(void) {
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("session.logout"), ==, 32);
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("window.restore_or_minimize"), ==, 48);
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("window.close"), ==, 64);
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("window.unknown"), ==, 64);
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("workspace.rename"), ==, 64);
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("animation.preview"), ==, 18);
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("outputs.list"), ==, 82);
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("launch.status"), ==, 8);
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("launch.begin"), ==, 50);
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("shortcut.bind"), ==, 22);
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("shortcut.unbind"), ==, 11);
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("privacy.state"), ==, 47);
    g_assert_cmpuint(gnoblin_api_operation_cli_minor("bogus.method"), ==, 8);
    g_assert_true(gnoblin_api_operation_has("window.unknown", GN_OP_VERSIONED));
    g_assert_true(gnoblin_api_operation_has("session.logout", GN_OP_VERSIONED));
    g_assert_false(gnoblin_api_operation_has("input.text_target", GN_OP_VERSIONED));
    g_assert_true(gnoblin_api_operation_has("window.thumbnail", GN_OP_WAITS));
    g_assert_false(gnoblin_api_operation_has("window.close", GN_OP_WAITS));
}

static void test_supervisor_fields(void) {
    g_assert_true(gnoblin_api_operation_has("window.focus", GN_OP_GRANT));
    g_assert_true(gnoblin_api_operation_has("window.begin_resize", GN_OP_GRANT));
    g_assert_true(gnoblin_api_operation_has("input.text_target", GN_OP_GRANT));
    g_assert_true(gnoblin_api_operation_has("window.snap_context", GN_OP_GRANT));
    g_assert_false(gnoblin_api_operation_has("window.close", GN_OP_GRANT));
    const char* const generation[] = {
        "window.snap.offer", "window.snap",       "window.snap_context", "window.set_visual",
        "window.set_effect", "settings.override", "override.revoke",     "idle.watch",
        "idle.cancel",       "input.grab",        "input.grab.renew",    "input.grab.release",
        "tween.play",        "tween.unknown",     "process.spawn",       "process.unknown"};
    for (gsize i = 0; i < G_N_ELEMENTS(generation); i++)
        g_assert_true(gnoblin_api_operation_has(generation[i], GN_OP_GENERATION));
    g_assert_false(gnoblin_api_operation_has("window.close", GN_OP_GENERATION));
    g_assert_false(gnoblin_api_operation_has("workspace.create", GN_OP_GENERATION));
    g_assert_true(gnoblin_api_operation_has("window.thumbnail", GN_OP_CLIENT_ID));
    g_assert_true(gnoblin_api_operation_has("auth.respond", GN_OP_CLIENT_ID));
    g_assert_true(gnoblin_api_operation_has("prompt.unknown", GN_OP_CLIENT_ID));
    g_assert_false(gnoblin_api_operation_has("session.lock", GN_OP_CLIENT_ID));
    g_assert_true(gnoblin_api_operation_has("windows.list", GN_OP_READ));
    g_assert_false(gnoblin_api_operation_has("window.close", GN_OP_READ));
    g_assert_true(gnoblin_api_operation_has("animation.inspect", GN_OP_ANIM_READ));
    g_assert_false(gnoblin_api_operation_has("animation.preview", GN_OP_ANIM_READ));
}

static gboolean takes_code(const GnoblinApiOperation* operation) {
    /* Record methods whose arguments are packed by code, not by a schema. */
    return operation->flags & GN_OP_GRANT || operation->id == GNOBLIN_OP_WINDOW_SET_EFFECT ||
           operation->id == GNOBLIN_OP_ANIMATION_INSPECT ||
           operation->id == GNOBLIN_OP_ANIMATION_PREVIEW;
}

static void test_lua_api_entries(void) {
    for (const LuaApiEntry* entry = lua_api; entry->path; entry++) {
        if (!entry->function)
            g_assert_nonnull(entry->operation);
        if (entry->operation && !entry->function && strchr(entry->path, ':'))
            g_assert_true(entry->arguments != NULL || takes_code(entry->operation));
        if (entry->arguments) {
            g_assert_nonnull(entry->operation);
            for (const LuaApiArg* argument = entry->arguments; argument->kind != LUA_ARG_END;
                 argument++)
                if (argument->kind == LUA_ARG_SIZE)
                    g_assert_cmpint(argument->limit, >, 0);
        }
        if (entry->operation && !strchr(entry->path, ':'))
            g_assert_true(lua_api_find_call(entry->operation->name) != NULL);
    }
    /* The wire-only operations are reachable by name. */
    g_assert_nonnull(lua_api_find_call("auth.begin"));
    g_assert_nonnull(lua_api_find_call("monitors.set_privacy_screen"));
    g_assert_nonnull(lua_api_find_call("window.minimize"));
    g_assert_null(lua_api_find_call("bogus.method"));
    /* Window.move declares x and y; the table call of window.move accepts those and the id. */
    const char* names[8];
    const GnoblinApiOperation* move = &gnoblin_api_operations[GNOBLIN_OP_WINDOW_MOVE];
    g_assert_cmpuint(lua_api_operation_fields(move, names, G_N_ELEMENTS(names)), ==, 3);
    g_assert_cmpstr(names[0], ==, "id");
    g_assert_cmpstr(names[1], ==, "x");
    g_assert_cmpstr(names[2], ==, "y");
    g_assert_null(names[3]);
    const GnoblinApiOperation* visual = &gnoblin_api_operations[GNOBLIN_OP_WINDOW_SET_VISUAL];
    g_assert_cmpuint(lua_api_operation_fields(visual, names, G_N_ELEMENTS(names)), ==, 7);
    g_assert_cmpstr(names[6], ==, "reset");
    const GnoblinApiOperation* thumbnail = &gnoblin_api_operations[GNOBLIN_OP_WINDOW_THUMBNAIL];
    g_assert_cmpint(lua_api_operation_argument(thumbnail, "width")->limit, ==, 480);
    g_assert_cmpint(lua_api_operation_argument(thumbnail, "height")->limit, ==, 320);
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/api-operations/hello-list", test_hello_list);
    g_test_add_func("/api-operations/rows", test_rows_are_consistent);
    g_test_add_func("/api-operations/gate-errors", test_gate_errors);
    g_test_add_func("/api-operations/cli-minor", test_cli_minor);
    g_test_add_func("/api-operations/supervisor-fields", test_supervisor_fields);
    g_test_add_func("/api-operations/lua-api-entries", test_lua_api_entries);
    return g_test_run();
}
