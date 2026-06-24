/*
 * gnoblin-shell: a minimal libmutter compositor (the gnoblin-shell spike).
 *
 * Embeds libmutter via the public MetaContext API and drives it with
 * GnoblinShellPlugin, instead of running gnome-shell. Modelled on mutter's own
 * executable entry point (src/core/mutter.c).
 *
 * Configuration is a plain file (see gnoblin-config.h), not GSettings. The
 * `[startup] exec` / `[startup] exec_per_output` lists name layer-shell apps to
 * launch with the shell (e.g. the gnoblin topbar/dock — exec_per_output spawns
 * one instance per monitor); the file is watched and re-read live, so adding an
 * entry launches it without restarting the compositor.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include <gio/gio.h>
#include <glib-unix.h>
#include <signal.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>

extern "C" {
#include <meta/main.h>
#include <meta/meta-backend.h>
#include <meta/meta-context.h>
#include <meta/meta-monitor-manager.h>
/* meta-monitor.h references MetaBacklight but its header pulls a private
 * include; we don't use the backlight API, so just forward-declare the type. */
typedef struct _MetaBacklight MetaBacklight;
#include <meta/meta-monitor.h>
#include <meta/util.h>
}

#include "gnoblin-config.h"
#include "gnoblin-control.h"
#include "gnoblin-gestures.h"
#include "gnoblin-input.h"
#include "gnoblin-output.h"
#include "gnoblin-shell-plugin.h"

#ifndef GNOBLIN_SHELL_VERSION
#define GNOBLIN_SHELL_VERSION "0.1"
#endif

static char** command_argv = NULL;
/* Keys: a global entry is the command string; a per-output entry is
 * "command\x1fconnector" so the same command spawns once per monitor. */
static GHashTable* autostarted;
static MetaContext* the_context = NULL; /* borrowed, for monitor enumeration */

/* Connector names of the currently-active monitors (e.g. "DP-1", "Meta-0").
 * These match the wl_output name a client resolves via `--output`. */
static GPtrArray* active_connectors(void) {
    GPtrArray* names = g_ptr_array_new_with_free_func(g_free);
    if (!the_context)
        return names;

    MetaBackend* backend = meta_context_get_backend(the_context);
    MetaMonitorManager* mm = meta_backend_get_monitor_manager(backend);
    for (GList* l = meta_monitor_manager_get_monitors(mm); l; l = l->next) {
        MetaMonitor* mon = (MetaMonitor*) l->data;
        if (!meta_monitor_is_active(mon))
            continue;
        const char* conn = meta_monitor_get_connector(mon);
        if (conn)
            g_ptr_array_add(names, g_strdup(conn));
    }
    return names;
}

static gboolean print_version(const char* option_name, const char* value, gpointer data,
                              GError** error) {
    g_print("gnoblin-shell %s\n", GNOBLIN_SHELL_VERSION);
    exit(EXIT_SUCCESS);
}

static GOptionEntry gnoblin_options[] = {
    {
        "version",
        0,
        G_OPTION_FLAG_NO_ARG,
        G_OPTION_ARG_CALLBACK, (gpointer) print_version,
        "Print version",
        NULL,
    },
    {
        G_OPTION_REMAINING,
        0,
        0,
        G_OPTION_ARG_STRING_ARRAY,
        &command_argv,
        NULL,
        "[-- COMMAND [ARGUMENT…]]",
    },
    {NULL, 0, 0, G_OPTION_ARG_NONE, NULL, NULL, NULL},
};

static gboolean shutting_down;

static gboolean on_sigterm(gpointer user_data) {
    shutting_down = TRUE; /* don't respawn autostart clients as we tear down */
    meta_context_terminate(META_CONTEXT(user_data));
    return G_SOURCE_REMOVE;
}

static void spawn_command(char** argv) {
    g_autoptr(GError) error = NULL;

    /* Children inherit WAYLAND_DISPLAY (mutter sets it) and connect to us. */
    if (!g_spawn_async(NULL, argv, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, NULL, &error))
        g_warning("gnoblin-shell: failed to launch '%s': %s", argv[0], error->message);
}

/* Per-launch state so a crashed autostart client can be respawned (the bars +
 * notifyd should survive a transient hiccup rather than vanish for the session),
 * with a guard so a client that crashes immediately on every start is given up
 * on instead of fork-bombed. */
typedef struct {
    char* cmd;
    char* connector; /* NULL for single-instance clients */
    gint64 spawned_us;
    int fast_fails;
} RespawnInfo;

static void respawn_info_free(RespawnInfo* r) {
    g_free(r->cmd);
    g_free(r->connector);
    g_free(r);
}

static gboolean spawn_entry_full(const char* cmd, const char* connector, int prior_fast_fails);

static char* autostart_key_for(const char* cmd, const char* connector) {
    if (connector)
        return g_strdup_printf("%s\x1f%s", cmd, connector);
    return g_strdup(cmd);
}

static gboolean strv_contains(GStrv values, const char* needle) {
    if (!values || !needle)
        return FALSE;

    for (int i = 0; values[i]; i++) {
        if (g_strcmp0(values[i], needle) == 0)
            return TRUE;
    }
    return FALSE;
}

static gboolean connector_is_active(const char* connector) {
    if (!connector)
        return TRUE;

    g_autoptr(GPtrArray) conns = active_connectors();
    for (guint i = 0; i < conns->len; i++) {
        if (g_strcmp0((const char*) g_ptr_array_index(conns, i), connector) == 0)
            return TRUE;
    }
    return FALSE;
}

static gboolean autostart_entry_is_current(const char* cmd, const char* connector) {
    if (!autostarted)
        return FALSE;

    g_autofree char* key = autostart_key_for(cmd, connector);
    if (!g_hash_table_contains(autostarted, key))
        return FALSE;

    if (connector) {
        g_auto(GStrv) per = gnoblin_config_get_list("startup", "exec_per_output");
        return strv_contains(per, cmd) && connector_is_active(connector);
    }

    g_auto(GStrv) apps = gnoblin_config_get_list("startup", "exec");
    return strv_contains(apps, cmd);
}

static void forget_autostarted(const char* cmd, const char* connector) {
    if (!autostarted)
        return;

    g_autofree char* key = autostart_key_for(cmd, connector);
    g_hash_table_remove(autostarted, key);
}

static gboolean autostart_exit_was_intentional(gint status) {
    if (!WIFSIGNALED(status))
        return FALSE;

    switch (WTERMSIG(status)) {
    case SIGTERM:
    case SIGINT:
    case SIGHUP:
    case SIGQUIT:
        return TRUE;
    default:
        return FALSE;
    }
}

static void on_client_exited(GPid pid, gint status, gpointer user_data) {
    RespawnInfo* r = (RespawnInfo*) user_data;
    gboolean fast = (g_get_monotonic_time() - r->spawned_us) < 2 * G_USEC_PER_SEC;
    int fails = fast ? r->fast_fails + 1 : 0;

    g_spawn_close_pid(pid);
    if (shutting_down || autostart_exit_was_intentional(status)) {
        respawn_info_free(r);
        return;
    }
    if (!autostart_entry_is_current(r->cmd, r->connector)) {
        respawn_info_free(r);
        return;
    }
    if (fails >= 5) {
        g_warning("gnoblin-shell: '%s' keeps exiting right after launch (status %d); "
                  "not respawning",
                  r->cmd, status);
        forget_autostarted(r->cmd, r->connector);
        respawn_info_free(r);
        return;
    }
    g_message("gnoblin-shell: autostart client '%s' exited (status %d) — respawning", r->cmd,
              status);
    if (!spawn_entry_full(r->cmd, r->connector, fails))
        forget_autostarted(r->cmd, r->connector);
    respawn_info_free(r);
}

/* Parse `cmd` and launch it, appending `--output <connector>` when given so the
 * client (a stateless renderer) binds to that monitor. Watches the child so it
 * is respawned if it dies unexpectedly. */
static gboolean spawn_entry_full(const char* cmd, const char* connector, int prior_fast_fails) {
    g_autoptr(GError) error = NULL;
    g_auto(GStrv) parsed = NULL;
    GPid pid = 0;

    if (!g_shell_parse_argv(cmd, NULL, &parsed, &error)) {
        g_warning("gnoblin-shell: bad autostart entry '%s': %s", cmd, error->message);
        return FALSE;
    }

    g_autoptr(GPtrArray) argv = g_ptr_array_new();
    for (char** p = parsed; *p; p++)
        g_ptr_array_add(argv, *p);
    if (connector) {
        g_ptr_array_add(argv, (gpointer) "--output");
        g_ptr_array_add(argv, (gpointer) connector);
    }
    g_ptr_array_add(argv, NULL);

    if (!g_spawn_async(NULL, (char**) argv->pdata, NULL,
                       (GSpawnFlags) (G_SPAWN_SEARCH_PATH | G_SPAWN_DO_NOT_REAP_CHILD), NULL, NULL,
                       &pid, &error)) {
        g_warning("gnoblin-shell: failed to launch '%s': %s", cmd, error->message);
        return FALSE;
    }

    RespawnInfo* r = g_new0(RespawnInfo, 1);
    r->cmd = g_strdup(cmd);
    r->connector = g_strdup(connector);
    r->spawned_us = g_get_monotonic_time();
    r->fast_fails = prior_fast_fails;
    g_child_watch_add(pid, on_client_exited, r);
    return TRUE;
}

static gboolean spawn_entry(const char* cmd, const char* connector) {
    return spawn_entry_full(cmd, connector, 0);
}

/* Launch any autostart entry we haven't launched yet (so a live config reload
 * or a monitor hotplug starts newly-needed clients without restarting the ones
 * already running). `[startup] exec` clients are single-instance; `[startup]
 * exec_per_output` clients get one instance per active monitor (topbar/dock/
 * wallpaper), each pinned to its output via --output. */
static void run_autostart(void) {
    g_auto(GStrv) apps = gnoblin_config_get_list("startup", "exec");
    if (apps) {
        for (int i = 0; apps[i]; i++) {
            if (g_hash_table_contains(autostarted, apps[i]))
                continue;
            if (spawn_entry(apps[i], NULL))
                g_hash_table_add(autostarted, autostart_key_for(apps[i], NULL));
        }
    }

    g_auto(GStrv) per = gnoblin_config_get_list("startup", "exec_per_output");
    if (per) {
        g_autoptr(GPtrArray) conns = active_connectors();
        for (int i = 0; per[i]; i++) {
            for (guint m = 0; m < conns->len; m++) {
                const char* conn = (const char*) g_ptr_array_index(conns, m);
                char* key = autostart_key_for(per[i], conn);
                if (g_hash_table_contains(autostarted, key)) {
                    g_free(key);
                    continue;
                }
                if (spawn_entry(per[i], conn))
                    g_hash_table_add(autostarted, key); /* hash takes ownership */
                else
                    g_free(key);
            }
        }
    }
}

/* Apply the [output] config once the main loop is running (async; it retries
 * internally until mutter has claimed DisplayConfig + laid out the monitors). */
static gboolean apply_output_once(gpointer user_data) {
    gnoblin_output_apply();
    return G_SOURCE_REMOVE;
}

/* On hotplug: forget per-output entries whose monitor is gone (its client has
 * already exited via the layer surface `closed` event), so a re-plugged monitor
 * respawns; then launch clients for any newly-arrived monitors. */
static void on_monitors_changed_autostart(MetaMonitorManager* mm, gpointer user_data) {
    g_autoptr(GPtrArray) conns = active_connectors();

    /* Re-apply the [output] config first so per-output clients respawn onto the
     * corrected layout. Re-applying an already-satisfied config is a no-op in
     * mutter, so this settles after one pass and does not loop. */
    gnoblin_output_apply();

    g_autoptr(GPtrArray) dead = g_ptr_array_new(); /* borrowed keys */
    GHashTableIter it;
    gpointer k;
    g_hash_table_iter_init(&it, autostarted);
    while (g_hash_table_iter_next(&it, &k, NULL)) {
        const char* key = (const char*) k;
        const char* sep = strchr(key, '\x1f');
        if (!sep)
            continue; /* global, never per-output */
        const char* conn = sep + 1;
        gboolean alive = FALSE;
        for (guint m = 0; m < conns->len; m++) {
            if (g_strcmp0((const char*) g_ptr_array_index(conns, m), conn) == 0) {
                alive = TRUE;
                break;
            }
        }
        if (!alive)
            g_ptr_array_add(dead, (gpointer) key);
    }
    for (guint i = 0; i < dead->len; i++)
        g_hash_table_remove(autostarted, g_ptr_array_index(dead, i));

    run_autostart();
}

static void on_config_changed(GFileMonitor* monitor, GFile* file, GFile* other,
                              GFileMonitorEvent event, gpointer user_data) {
    if (event != G_FILE_MONITOR_EVENT_CHANGES_DONE_HINT && event != G_FILE_MONITOR_EVENT_CREATED &&
        event != G_FILE_MONITOR_EVENT_RENAMED)
        return;

    g_message("gnoblin-shell: config changed, reloading");
    gnoblin_config_reload();
    run_autostart();
    gnoblin_shell_plugin_reload_appearance();
    gnoblin_input_apply();
    if (user_data)
        gnoblin_control_reload_keybindings(META_DISPLAY(user_data));
}

static void watch_config(MetaDisplay* display) {
    g_autoptr(GFile) file = g_file_new_for_path(gnoblin_config_path());
    static GFileMonitor* monitor; /* kept alive for the process lifetime */

    monitor = g_file_monitor_file(file, G_FILE_MONITOR_WATCH_MOVES, NULL, NULL);
    if (monitor)
        g_signal_connect(monitor, "changed", G_CALLBACK(on_config_changed), display);
}

int main(int argc, char** argv) {
    g_autoptr(MetaContext) context = NULL;
    g_autoptr(GError) error = NULL;

    /* Load config before start so the compositor + protocols see it. */
    gnoblin_config_reload();

    context = meta_create_context("Gnoblin");

    meta_context_add_option_entries(context, gnoblin_options, NULL);

    if (!meta_context_configure(context, &argc, &argv, &error)) {
        g_printerr("gnoblin-shell: failed to configure: %s\n", error->message);
        return EXIT_FAILURE;
    }

    meta_context_set_plugin_gtype(context, GNOBLIN_TYPE_SHELL_PLUGIN);

    /* Make gnoblin's config the source of truth for keybindings — disable
     * mutter's built-ins before it loads them during setup. */
    gnoblin_control_take_over_keybindings();

    g_unix_signal_add(SIGTERM, on_sigterm, context);

    if (!meta_context_setup(context, &error)) {
        g_printerr("gnoblin-shell: failed to setup: %s\n", error->message);
        return EXIT_FAILURE;
    }

    if (!meta_context_start(context, &error)) {
        g_printerr("gnoblin-shell: failed to start: %s\n", error->message);
        return EXIT_FAILURE;
    }

    meta_context_notify_ready(context);

    /* A Wayland compositor opens many fds (clients, inotify, DRM); raise the
     * soft limit to the hard limit, as mutter's own main() does. */
    if (meta_context_get_compositor_type(context) == META_COMPOSITOR_TYPE_WAYLAND)
        meta_context_raise_rlimit_nofile(context, NULL);

    autostarted = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    the_context = context;

    /* Respawn per-output clients when monitors come and go. */
    {
        MetaBackend* backend = meta_context_get_backend(context);
        MetaMonitorManager* mm = meta_backend_get_monitor_manager(backend);
        g_signal_connect(mm, "monitors-changed",
                         G_CALLBACK(on_monitors_changed_autostart), NULL);
    }

    /* Push the [input] config onto GSettings before clients/devices settle. */
    gnoblin_input_apply();

    /* Start decoding touchpad gestures (3/4-finger swipe, pinch) into actions. */
    gnoblin_gestures_init(meta_context_get_display(context));

    /* Apply the [output] config once the main loop is up and mutter has claimed
     * org.gnome.Mutter.DisplayConfig on the bus (a synchronous call here, before
     * the loop runs, would race the name acquisition). Hotplug is handled by the
     * monitors-changed handler above. */
    g_timeout_add_seconds(1, apply_output_once, NULL);

    run_autostart();
    watch_config(meta_context_get_display(context));

    /* Also honour an explicit `-- COMMAND` (used by the devkit runner). */
    if (command_argv)
        spawn_command(command_argv);

    if (!meta_context_run_main_loop(context, &error)) {
        g_printerr("gnoblin-shell: terminated with a failure: %s\n", error->message);
        return EXIT_FAILURE;
    }

    return EXIT_SUCCESS;
}
