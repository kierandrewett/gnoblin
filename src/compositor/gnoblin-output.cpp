/*
 * gnoblin-shell: output config -> org.gnome.Mutter.DisplayConfig bridge.
 * See gnoblin-output.h.
 *
 * All D-Bus calls are ASYNCHRONOUS: DisplayConfig is exported by mutter on the
 * same process + main-loop thread as gnoblin-shell, so a *synchronous* call to
 * it would deadlock (the method handler can't run while the caller blocks the
 * loop waiting for the reply). Async send returns immediately, the loop services
 * the handler, and our callback fires with the result.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-output.h"

#include <gio/gio.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "gnoblin-config.h"
#include "gnoblin-output-spec.h"

#define OUTPUT "output"
#define DC_NAME "org.gnome.Mutter.DisplayConfig"
#define DC_PATH "/org/gnome/Mutter/DisplayConfig"
#define METHOD_TEMPORARY 1u /* MetaMonitorsConfigMethod: apply, don't persist */
#define MAX_RETRIES 8

/* Find the mode id for `connector` matching `spec` (or its current mode), and
 * whether `spec`'s scale is supported by that mode. Returns a newly-allocated
 * id, or NULL if the connector has no usable mode. */
static char* pick_mode(GVariant* monitors, const char* connector, const GnoblinOutputSpec* spec,
                       gboolean* scale_ok, char** current_id) {
    GVariantIter it;
    GVariant* mon;
    char* chosen = NULL;

    *scale_ok = !spec->has_scale; /* nothing to check if scale unset */
    if (current_id)
        *current_id = NULL;
    g_variant_iter_init(&it, monitors);
    while ((mon = g_variant_iter_next_value(&it))) {
        g_autoptr(GVariant) info = g_variant_get_child_value(mon, 0);
        g_autoptr(GVariant) modes = g_variant_get_child_value(mon, 1);
        const char* conn = NULL;
        GVariantIter mit;
        GVariant* mode;

        g_variant_get_child(info, 0, "&s", &conn);
        if (g_strcmp0(conn, connector) != 0) {
            g_variant_unref(mon);
            continue;
        }

        g_variant_iter_init(&mit, modes);
        while ((mode = g_variant_iter_next_value(&mit))) {
            const char* id = NULL;
            int w = 0, h = 0;
            double refresh = 0, pref = 0;
            g_autoptr(GVariant) scales = NULL;
            g_autoptr(GVariant) mprops = NULL;
            gboolean is_current = FALSE;
            gboolean match;

            g_variant_get(mode, "(&siidd@ad@a{sv})", &id, &w, &h, &refresh, &pref, &scales,
                          &mprops);
            g_variant_lookup(mprops, "is-current", "b", &is_current);
            if (is_current && current_id && !*current_id)
                *current_id = g_strdup(id);

            match = spec->has_mode
                        ? (w == spec->mode_w && h == spec->mode_h &&
                           (spec->mode_refresh == 0 || fabs(refresh - spec->mode_refresh) < 1.0))
                        : is_current;
            if (match && !chosen) {
                chosen = g_strdup(id);
                if (spec->has_scale) {
                    GVariantIter sit;
                    double s;
                    g_variant_iter_init(&sit, scales);
                    while (g_variant_iter_next(&sit, "d", &s)) {
                        if (fabs(s - spec->scale) < 0.01)
                            *scale_ok = TRUE;
                    }
                }
            }
            g_variant_unref(mode);
        }
        g_variant_unref(mon);
    }
    return chosen;
}

static void on_apply_done(GObject* source, GAsyncResult* res, gpointer user_data) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source), res, &error);

    if (!reply)
        g_warning("gnoblin-output: ApplyMonitorsConfig failed: %s", error->message);
    else
        g_message("gnoblin-output: applied output config");
}

static void schedule_retry(void); /* fwd */

/* GetCurrentState reply: build the new layout from current + [output] overrides
 * and apply it (async). */
static void on_get_state(GObject* source, GAsyncResult* res, gpointer user_data) {
    GDBusConnection* bus = G_DBUS_CONNECTION(source);
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) state = g_dbus_connection_call_finish(bus, res, &error);
    g_autoptr(GVariant) monitors = NULL;
    g_autoptr(GVariant) logical = NULL;
    GVariantBuilder lm;
    GVariantIter lit;
    GVariant* log_mon;
    guint serial;
    gboolean any = FALSE;

    if (!state) {
        /* DisplayConfig not on the bus yet — retry a few times at startup. */
        schedule_retry();
        return;
    }
    monitors = g_variant_get_child_value(state, 1);
    logical = g_variant_get_child_value(state, 2);
    g_variant_get_child(state, 0, "u", &serial);

    if (g_variant_n_children(logical) == 0) {
        schedule_retry(); /* monitors not laid out yet */
        return;
    }

    g_variant_builder_init(&lm, G_VARIANT_TYPE("a(iiduba(ssa{sv}))"));
    g_variant_iter_init(&lit, logical);
    while ((log_mon = g_variant_iter_next_value(&lit))) {
        int x, y;
        double scale;
        guint transform;
        gboolean primary;
        g_autoptr(GVariant) lmons = NULL;
        g_autoptr(GVariant) lprops = NULL;
        const char* connector = NULL;
        g_autofree char* spec_str = NULL;
        GnoblinOutputSpec spec;
        g_autofree char* mode_id = NULL;
        gboolean scale_ok = TRUE;
        GVariantBuilder mons;

        g_variant_get(log_mon, "(iidub@a(ssss)@a{sv})", &x, &y, &scale, &transform, &primary,
                      &lmons, &lprops);
        if (g_variant_n_children(lmons) > 0)
            g_variant_get_child(lmons, 0, "(&ssss)", &connector, NULL, NULL, NULL);

        spec_str = connector ? gnoblin_config_get_string(OUTPUT, connector) : NULL;
        if (spec_str)
            gnoblin_output_parse_spec(spec_str, &spec);
        else
            memset(&spec, 0, sizeof(spec));

        if (spec_str && spec.disable) {
            g_message("gnoblin-output: %s disabled", connector);
            any = TRUE;
            g_variant_unref(log_mon);
            continue; /* omit from the layout == disable it */
        }

        g_autofree char* current_id = NULL;
        mode_id = pick_mode(monitors, connector, &spec, &scale_ok, &current_id);
        if (!mode_id) {
            g_warning("gnoblin-output: no mode for %s, leaving as-is", connector);
            g_variant_unref(log_mon);
            continue;
        }
        if (spec.has_scale && !scale_ok) {
            g_warning("gnoblin-output: scale %.2f unsupported on %s; keeping %.2f", spec.scale,
                      connector, scale);
            spec.has_scale = FALSE;
        }
        /* Only request an apply if some field actually differs from the current
         * state — otherwise mutter re-emits monitors-changed for an identical
         * config and we'd spin forever. */
        if (spec_str) {
            if ((spec.has_position && (spec.pos_x != x || spec.pos_y != y)) ||
                (spec.has_scale && fabs(spec.scale - scale) > 0.01) ||
                (spec.has_transform && spec.transform != transform) ||
                (spec.primary && !primary) ||
                (spec.has_mode && g_strcmp0(mode_id, current_id) != 0))
                any = TRUE;
        }

        g_variant_builder_init(&mons, G_VARIANT_TYPE("a(ssa{sv})"));
        g_variant_builder_add(&mons, "(ss@a{sv})", connector, mode_id,
                              g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
        g_variant_builder_add(&lm, "(iidub@a(ssa{sv}))", spec.has_position ? spec.pos_x : x,
                              spec.has_position ? spec.pos_y : y,
                              spec.has_scale ? spec.scale : scale,
                              spec.has_transform ? spec.transform : transform,
                              spec.primary ? TRUE : primary, g_variant_builder_end(&mons));
        g_variant_unref(log_mon);
    }

    if (!any) {
        g_variant_builder_clear(&lm);
        return; /* readable, but none of the monitors had an override */
    }

    g_dbus_connection_call(
        bus, DC_NAME, DC_PATH, DC_NAME, "ApplyMonitorsConfig",
        g_variant_new("(uu@a(iiduba(ssa{sv}))@a{sv})", serial, METHOD_TEMPORARY,
                      g_variant_builder_end(&lm),
                      g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0)),
        NULL, G_DBUS_CALL_FLAGS_NONE, -1, NULL, on_apply_done, NULL);
}

void gnoblin_output_apply(void) {
    g_auto(GStrv) connectors = gnoblin_config_get_keys(OUTPUT);
    g_autoptr(GDBusConnection) bus = NULL;
    g_autoptr(GError) error = NULL;

    if (!connectors || !connectors[0])
        return; /* nothing configured — never touch the display layout */

    /* g_bus_get_sync only connects to the bus daemon (no round-trip to mutter),
     * so it doesn't deadlock; the method calls below are async. */
    bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    if (!bus) {
        g_warning("gnoblin-output: no session bus: %s", error->message);
        return;
    }
    g_dbus_connection_call(bus, DC_NAME, DC_PATH, DC_NAME, "GetCurrentState", NULL, NULL,
                           G_DBUS_CALL_FLAGS_NONE, -1, NULL, on_get_state, NULL);
}

static gboolean retry_cb(gpointer user_data) {
    gnoblin_output_apply();
    return G_SOURCE_REMOVE;
}

/* Re-kick the apply after a short delay, bounded so we never spin forever if the
 * configured connector is absent. */
static void schedule_retry(void) {
    static int retries;

    if (retries++ < MAX_RETRIES)
        g_timeout_add_seconds(1, retry_cb, NULL);
}
