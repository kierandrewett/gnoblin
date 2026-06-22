/*
 * gnoblin-shell: role-based client exec + feature toggles. See gnoblin-roles.h.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-roles.h"

#include <gio/gio.h>

extern "C" {
#include <meta/display.h>
#include <meta/meta-backend.h>
#include <meta/meta-context.h>
#include <meta/meta-logical-monitor.h>
#include <meta/meta-monitor-manager.h>
/* meta-monitor.h references MetaBacklight but pulls a private include; we don't
 * use the backlight API, so forward-declare the type (see gnoblin-shell.cpp). */
typedef struct _MetaBacklight MetaBacklight;
#include <meta/meta-monitor.h>
#include <meta/window.h>
}

#include "gnoblin-config.h"

char* gnoblin_role_command(const char* role) {
    return gnoblin_config_get_string("roles", role);
}

/* The connector name (== wl_output name) of logical monitor `idx`, or NULL. */
static const char* connector_for_monitor(MetaDisplay* display, int idx) {
    if (idx < 0)
        return NULL;
    MetaContext* ctx = meta_display_get_context(display);
    MetaBackend* backend = meta_context_get_backend(ctx);
    MetaMonitorManager* mm = meta_backend_get_monitor_manager(backend);
    for (GList* l = meta_monitor_manager_get_logical_monitors(mm); l; l = l->next) {
        MetaLogicalMonitor* lm = (MetaLogicalMonitor*) l->data;
        if (meta_logical_monitor_get_number(lm) != idx)
            continue;
        GList* mons = meta_logical_monitor_get_monitors(lm);
        if (mons)
            return meta_monitor_get_connector((MetaMonitor*) mons->data);
    }
    return NULL;
}

/* The connector name of the monitor containing the point (x,y) — matches the
 * wl_output name the spawned client resolves via --output. On success also
 * writes the monitor origin so the caller can translate a global anchor point
 * into that output's local coordinates. Returns NULL if it can't be resolved
 * (then the client binds the compositor's default output, as before). */
static const char* connector_for_point(MetaDisplay* display, int x, int y, int* ox, int* oy) {
    MtkRectangle point = {x, y, 1, 1};
    int idx = meta_display_get_monitor_index_for_rect(display, &point);
    if (idx < 0)
        return NULL;

    MtkRectangle geom;
    meta_display_get_monitor_geometry(display, idx, &geom);
    *ox = geom.x;
    *oy = geom.y;
    return connector_for_monitor(display, idx);
}

void gnoblin_export_active_output(MetaDisplay* display) {
    const char* conn = connector_for_monitor(display, meta_display_get_current_monitor(display));
    if (conn)
        g_setenv("GNOBLIN_ACTIVE_OUTPUT", conn, TRUE);
}

gboolean gnoblin_feature_enabled(const char* feature, gboolean dflt) {
    return gnoblin_config_get_bool("features", feature, dflt);
}

static void add_flag(GPtrArray* argv, const char* flag, const char* value) {
    g_ptr_array_add(argv, g_strdup(flag));
    g_ptr_array_add(argv, g_strdup(value));
}

gboolean gnoblin_role_spawn(const char* role, MetaWindow* window, int x, int y,
                            const char* reason) {
    g_autofree char* command = gnoblin_role_command(role);
    g_auto(GStrv) base = NULL;
    g_autoptr(GError) error = NULL;
    GPtrArray* argv;
    gboolean ok;
    int i;

    if (!command || command[0] == '\0')
        return FALSE;

    if (!g_shell_parse_argv(command, NULL, &base, &error)) {
        g_warning("gnoblin: bad [roles] %s = '%s': %s", role, command, error->message);
        return FALSE;
    }

    /* base command + appended context flags. */
    argv = g_ptr_array_new_with_free_func(g_free);
    for (i = 0; base[i]; i++)
        g_ptr_array_add(argv, g_strdup(base[i]));

    add_flag(argv, "--role", role);
    if (window) {
        g_autofree char* id = g_strdup_printf("%" G_GUINT64_FORMAT, meta_window_get_id(window));
        add_flag(argv, "--window", id);
    }

    /* Pin the client to the monitor under the anchor point and pass the anchor
     * in that output's LOCAL coordinates — otherwise, on multi-monitor, a
     * full-screen role surface (e.g. the window menu) binds the primary output
     * while its global x/y anchor lands off-screen. */
    int lx = x, ly = y;
    if (window) {
        MetaDisplay* display = meta_window_get_display(window);
        int ox = 0, oy = 0;
        const char* connector = display ? connector_for_point(display, x, y, &ox, &oy) : NULL;
        if (connector) {
            lx = x - ox;
            ly = y - oy;
            add_flag(argv, "--output", connector);
        }
    }
    {
        g_autofree char* xs = g_strdup_printf("%d", lx);
        g_autofree char* ys = g_strdup_printf("%d", ly);
        add_flag(argv, "--x", xs);
        add_flag(argv, "--y", ys);
    }
    if (reason)
        add_flag(argv, "--reason", reason);
    g_ptr_array_add(argv, NULL);

    /* Children inherit WAYLAND_DISPLAY (mutter sets it) and connect to us. */
    ok = g_spawn_async(NULL, (char**) argv->pdata, NULL,
                       G_SPAWN_SEARCH_PATH, NULL, NULL,
                       NULL, &error);
    if (!ok)
        g_warning("gnoblin: failed to spawn [roles] %s: %s", role, error->message);

    g_ptr_array_free(argv, TRUE);
    return ok;
}
