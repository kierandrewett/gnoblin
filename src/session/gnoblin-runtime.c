#define _GNU_SOURCE
#include "../native-control/gnoblin-runtime-protocol.h"
#include "../native-control/gnoblin-input-config.h"
#include "gnoblin-runtime-spawn.h"
#include "gnoblin-autostart.h"
#include "../config/gnoblin-config.h"

#include <errno.h>
#include <fcntl.h>
#include <spawn.h>
#include <signal.h>
#include <stdlib.h>
#include <poll.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>

#include <glib-unix.h>

extern char** environ;

#define RUNTIME_FD 198
#define WORKER_READY_FD 199
#define WORKER_HOST_FD 200
#define GUARDIAN_STATUS_FD 201
#define GUARDIAN_AUTOSTART_FD 202
#define WORKER_GUARDIAN_STATUS_FD 203
#define STARTUP_TIMEOUT_MS 10000
#define SUSPEND_TIMEOUT_MS 10000
#define WORKER_MAX_RESTARTS 5
#define WORKER_STABLE_RESET_MS 60000
#define EXIT_RESUME_REJECTED 75
#define GUARDIAN_STATUS_STARTED 1
#define GUARDIAN_STATUS_READY 2
#define GUARDIAN_STATUS_LOGOUT 3
#define SESSION_SUPERVISOR_BUS_NAME "org.gnoblin.SessionSupervisor"
#define PORTAL_BACKEND_BUS_NAME "org.freedesktop.impl.portal.desktop.gnoblin"
#define PORTAL_LIFECYCLE_INTERFACE "org.gnoblin.Portal.InhibitLifecycle"
#ifndef GNOBLIN_DEFAULT_COMPOSITOR
#define GNOBLIN_DEFAULT_COMPOSITOR "/usr/libexec/gnoblin-mutter"
#endif

typedef struct {
    GMainLoop* loop;
    GnoblinRuntimeWriter* writer;
    GnoblinRuntimeReader* reader;
    GHashTable* state_revisions;
    GHashTable* pending_logout_ids;
    GHashTable* pending_operation_ids;
    GQueue policy_events;
    GQueue deferred_packets;
    struct _RuntimeReload* pending_reload;
    guint reload_progress_id;
    guint deferred_packets_idle_id;
    guint deferred_callback_idle_id;
    guint64 event_sequence;
    guint64 current_client_id;
    GPid compositor_pid;
    int channel_fd;
    gboolean ready;
    gboolean failed;
    gboolean logout_requested;
    gboolean resume_worker;
    gboolean send_initial_autostart;
    gboolean resume_rejected;
    int ready_fd;
    int host_control_fd;
    int guardian_status_fd;
    GVariant* initial_document;
    guint startup_timeout_id;
    guint child_watch_id;
    guint write_watch_id;
    guint policy_event_idle_id;
    int signal_exit_status;
    gint exit_status;
} Runtime;

typedef struct {
    const char* event;
    GVariant* payload;
} RuntimePolicyEvent;

typedef struct _RuntimeReload {
    guint64 request_id;
    guint64 revision;
    guint64 generation;
    char* path;
    GVariant* document;
    guint timeout_id;
    gboolean sent;
} RuntimeReload;

typedef struct {
    GnoblinRuntimePacketType type;
    guint64 request_id;
    GVariant* payload;
} RuntimeDeferredPacket;

static char* session_prefix;

static gboolean send_initial_autostart(int fd, GVariant* document, GVariant* hello, GError** error);
static gboolean guardian_send_status(int fd, guint8 status);

static GDBusConnection* claim_session_lifecycle_bus(void) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) connection = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    if (!connection) {
        g_debug("gnoblin: portal end-session notifications unavailable: %s", error->message);
        return NULL;
    }

    g_autoptr(GVariant) reply = g_dbus_connection_call_sync(
        connection, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        "RequestName", g_variant_new("(su)", SESSION_SUPERVISOR_BUS_NAME, 4u),
        G_VARIANT_TYPE("(u)"), G_DBUS_CALL_FLAGS_NONE, 2000, NULL, &error);
    if (!reply) {
        g_debug("gnoblin: portal end-session notifications unavailable: %s", error->message);
        return NULL;
    }

    guint32 result = 0;
    g_variant_get(reply, "(u)", &result);
    if (result != 1) {
        g_debug("gnoblin: another session owns %s", SESSION_SUPERVISOR_BUS_NAME);
        return NULL;
    }

    return g_steal_pointer(&connection);
}

static void notify_portal_session_ending(GDBusConnection* connection) {
    if (!connection)
        return;

    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply = g_dbus_connection_call_sync(
        connection, PORTAL_BACKEND_BUS_NAME, "/org/freedesktop/portal/desktop",
        PORTAL_LIFECYCLE_INTERFACE, "PrepareForEnd", NULL, G_VARIANT_TYPE("()"),
        G_DBUS_CALL_FLAGS_NONE, 2000, NULL, &error);
    if (!reply)
        g_debug("gnoblin: portal end-session notification was not completed: %s", error->message);
}

static gboolean run_command(const char* const argv[], gboolean required) {
    g_autoptr(GError) error = NULL;
    gint status = 0;
    if (!g_spawn_sync(NULL, (char**)argv, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, NULL, NULL,
                      &status, &error)) {
        if (required)
            g_printerr("gnoblin: could not run %s: %s\n", argv[0], error->message);
        return FALSE;
    }
    gboolean ok = WIFEXITED(status) && WEXITSTATUS(status) == 0;
    if (!ok && required)
        g_printerr("gnoblin: %s failed\n", argv[0]);
    return ok;
}

static gboolean run_activation_update(const char* const extra[]) {
    const char* argv[32];
    guint n = 0;
    argv[n++] = "dbus-update-activation-environment";
    argv[n++] = "--systemd";
    for (guint i = 0; extra[i] && n < G_N_ELEMENTS(argv) - 1; i++)
        argv[n++] = extra[i];
    argv[n] = NULL;
    return run_command(argv, TRUE);
}

static void reset_online_accounts_services(void) {
    const char* list[] = {"systemctl",
                          "--user",
                          "list-units",
                          "--all",
                          "--plain",
                          "--no-legend",
                          "dbus-*-org.gnome.OnlineAccounts@*.service",
                          "dbus-*-org.gnome.Identity@*.service",
                          NULL};
    gchar* output = NULL;
    gint status = 0;
    if (!g_spawn_sync(NULL, (char**)list, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, &output, NULL,
                      &status, NULL) ||
        !WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        g_free(output);
        return;
    }
    g_auto(GStrv) lines = g_strsplit(output, "\n", -1);
    g_free(output);
    for (guint i = 0; lines[i]; i++) {
        g_auto(GStrv) fields = g_strsplit_set(lines[i], " \t", 0);
        const char* unit = fields[0];
        if (!unit || !*unit ||
            (!g_pattern_match_simple("dbus-*-org.gnome.OnlineAccounts@*.service", unit) &&
             !g_pattern_match_simple("dbus-*-org.gnome.Identity@*.service", unit)))
            continue;
        const char* stop[] = {"systemctl", "--user", "stop", unit, NULL};
        run_command(stop, TRUE);
    }
}

static void session_cleanup(void) {
    const char* stop[] = {
        "systemctl", "--user", "stop", "gnoblin-session.target", "graphical-session.target", NULL};
    const char* clear[] = {"systemctl",
                           "--user",
                           "unset-environment",
                           "GNOME_SHELL_SESSION_MODE",
                           "XDG_CURRENT_DESKTOP",
                           "XDG_SESSION_DESKTOP",
                           "XDG_SESSION_CLASS",
                           "XDG_SESSION_TYPE",
                           "WAYLAND_DISPLAY",
                           "DISPLAY",
                           "XAUTHORITY",
                           "GNOME_SETUP_DISPLAY",
                           NULL};
    const char* unset[] = {"GNOME_SHELL_SESSION_MODE=",
                           "XDG_CURRENT_DESKTOP=",
                           "XDG_SESSION_DESKTOP=",
                           "XDG_SESSION_CLASS=",
                           "XDG_SESSION_TYPE=",
                           "WAYLAND_DISPLAY=",
                           "DISPLAY=",
                           "XAUTHORITY=",
                           "GNOME_SETUP_DISPLAY=",
                           NULL};
    run_command(stop, FALSE);
    run_activation_update(unset);
    run_command(clear, FALSE);
    g_clear_pointer(&session_prefix, g_free);
}

static gboolean prepare_session_environment(void) {
    g_autofree char* exe = g_file_read_link("/proc/self/exe", NULL);
    if (!exe) {
        g_printerr("gnoblin: could not locate installed executable\n");
        return FALSE;
    }
    g_autofree char* bin_dir = g_path_get_dirname(exe);
    session_prefix = g_path_get_dirname(bin_dir);
    if (!g_str_equal(g_getenv("XDG_SESSION_TYPE") ?: "", "wayland") ||
        !g_getenv("XDG_SESSION_ID") || !*g_getenv("XDG_SESSION_ID") ||
        (g_getenv("WAYLAND_DISPLAY") && *g_getenv("WAYLAND_DISPLAY")) ||
        (g_getenv("DISPLAY") && *g_getenv("DISPLAY"))) {
        g_printerr("gnoblin: start it from a fresh Wayland login session\n");
        return FALSE;
    }
    const char* active[] = {
        "systemctl", "--user", "is-active", "--quiet", "graphical-session.target", NULL};
    if (run_command(active, FALSE)) {
        g_printerr("gnoblin: another graphical session is already active\n");
        return FALSE;
    }
    g_unsetenv("XAUTHORITY");
    g_unsetenv("GNOME_SETUP_DISPLAY");
    g_unsetenv("GSETTINGS_SCHEMA_DIR");
    g_unsetenv("GI_TYPELIB_PATH");
    g_setenv("GNOBLIN_PREFIX", session_prefix, TRUE);
    g_autofree char* libdir_file = g_build_filename(session_prefix, "libexec/gnoblin-libdir", NULL);
    g_autofree char* libdir = NULL;
    g_file_get_contents(libdir_file, &libdir, NULL, NULL);
    if (libdir)
        g_strchomp(libdir);
    if (!libdir || !*libdir)
        libdir = g_strdup("lib64");
    g_setenv("GNOBLIN_LIBDIR", libdir, TRUE);
    const char* old_ld = g_getenv("LD_LIBRARY_PATH");
    g_autofree char* private_libs = g_build_filename(session_prefix, libdir, NULL);
    const char* api = g_getenv("GNOBLIN_MUTTER_API");
    g_autofree char* mutter_dir = g_strdup_printf("mutter-%s", api && *api ? api : "51");
    g_autofree char* mutter_libs = g_build_filename(private_libs, mutter_dir, NULL);
    g_autofree char* cxx_file = g_build_filename(session_prefix, "libexec/gnoblin-cxx-lib", NULL);
    g_autofree char* cxx_lib = NULL;
    g_file_get_contents(cxx_file, &cxx_lib, NULL, NULL);
    if (cxx_lib)
        g_strchomp(cxx_lib);
    g_autofree char* ld = cxx_lib && *cxx_lib
                              ? g_strdup_printf("%s:%s:%s", private_libs, mutter_libs, cxx_lib)
                              : g_strdup_printf("%s:%s%s%s", private_libs, mutter_libs,
                                                old_ld && *old_ld ? ":" : "", old_ld ?: "");
    g_setenv("LD_LIBRARY_PATH", ld, TRUE);
    g_autofree char* path =
        g_strdup_printf("%s/bin:%s", session_prefix, g_getenv("PATH") ?: "/usr/bin:/bin");
    g_setenv("PATH", path, TRUE);
    g_autofree char* data_dirs = g_strdup_printf(
        "%s/share:%s", session_prefix, g_getenv("XDG_DATA_DIRS") ?: "/usr/local/share:/usr/share");
    g_setenv("XDG_DATA_DIRS", data_dirs, TRUE);
    g_setenv("XDG_CURRENT_DESKTOP", "Gnoblin", TRUE);
    g_setenv("XDG_SESSION_DESKTOP", "gnoblin", TRUE);
    const char* session_id = g_getenv("XDG_SESSION_ID");
    if (session_id) {
        const char* loginctl[] = {"loginctl",         "show-session", session_id,
                                  "--property=Class", "--value",      NULL};
        gchar* out = NULL;
        gint status = 0;
        if (g_spawn_sync(NULL, (char**)loginctl, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, &out, NULL,
                         &status, NULL) &&
            WIFEXITED(status) && WEXITSTATUS(status) == 0 && out && *g_strstrip(out))
            g_setenv("XDG_SESSION_CLASS", g_strstrip(out), TRUE);
        g_free(out);
    }
    g_autofree char* example =
        g_build_filename(session_prefix, "share/gnoblin/init.lua.example", NULL);
    const char* seed[] = {"gnoblin-seed-config", example, NULL};
    g_autofree char* seed_path =
        g_build_filename(session_prefix, "libexec/gnoblin-seed-config", NULL);
    seed[0] = seed_path;
    if (!run_command(seed, TRUE))
        return FALSE;
    return TRUE;
}

static gboolean activate_session(void) {
    atexit(session_cleanup);
    const char* clear_display[] = {
        "WAYLAND_DISPLAY=", "DISPLAY=", "XAUTHORITY=", "GNOME_SETUP_DISPLAY=", NULL};
    if (!run_activation_update(clear_display))
        return FALSE;
    const char* unset_display[] = {"systemctl",           "--user",  "unset-environment",
                                   "WAYLAND_DISPLAY",     "DISPLAY", "XAUTHORITY",
                                   "GNOME_SETUP_DISPLAY", NULL};
    run_command(unset_display, FALSE);
    const char* sync[] = {"GNOME_SHELL_SESSION_MODE",
                          "XDG_CONFIG_HOME",
                          "XDG_CURRENT_DESKTOP",
                          "XDG_SESSION_DESKTOP",
                          "XDG_SESSION_CLASS",
                          "XDG_SESSION_TYPE",
                          "WAYLAND_DISPLAY",
                          "DISPLAY",
                          "XAUTHORITY",
                          NULL};
    g_unsetenv("GNOME_SHELL_SESSION_MODE");
    const char* clear_session_mode[] = {"systemctl", "--user", "unset-environment",
                                        "GNOME_SHELL_SESSION_MODE", NULL};
    run_command(clear_session_mode, FALSE);
    if (!run_activation_update(sync))
        return FALSE;
    reset_online_accounts_services();
    const char* stop_old[] = {"systemctl", "--user", "stop",
                              "org.gnome.SettingsDaemon.ScreensaverProxy.service", NULL};
    run_command(stop_old, FALSE);
    const char* clear_private[] = {"systemctl",       "--user",          "unset-environment",
                                   "GNOBLIN_PREFIX",  "GNOBLIN_LIBDIR",  "GSETTINGS_SCHEMA_DIR",
                                   "LD_LIBRARY_PATH", "GI_TYPELIB_PATH", NULL};
    run_command(clear_private, FALSE);
    const char* reload[] = {"systemctl", "--user", "daemon-reload", NULL};
    const char* start[] = {
        "systemctl", "--user", "start", "graphical-session.target", "gnoblin-session.target", NULL};
    if (!run_command(reload, TRUE) || !run_command(start, TRUE))
        return FALSE;
    return TRUE;
}

typedef struct {
    Runtime* runtime;
    int signal_number;
} RuntimeSignal;

static void runtime_fail(Runtime* runtime, const char* message) {
    if (!runtime || runtime->failed)
        return;
    runtime->failed = TRUE;
    g_printerr("gnoblin: runtime supervisor: %s\n", message);
    if (runtime->loop)
        g_main_loop_quit(runtime->loop);
}

static GVariant* config_packet_payload(GVariant* document, guint64 revision, guint64 generation) {
    g_auto(GStrv) runtime_events = gnoblin_config_runtime_events();
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "document_version", g_variant_new_uint32(1));
    g_variant_builder_add(&builder, "{sv}", "settings_revision", g_variant_new_uint64(revision));
    g_variant_builder_add(&builder, "{sv}", "runtime_generation", g_variant_new_uint64(generation));
    g_variant_builder_add(&builder, "{sv}", "document", document);
    g_variant_builder_add(&builder, "{sv}", "runtime_events",
                          g_variant_new_strv((const char* const*)runtime_events, -1));
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static GVariant* resume_packet_payload(GVariant* document, guint64 revision, guint64 generation,
                                       guint64 operation_id_watermark) {
    g_auto(GStrv) runtime_events = gnoblin_config_runtime_events();
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "document", document);
    g_variant_builder_add(&builder, "{sv}", "settings_revision", g_variant_new_uint64(revision));
    g_variant_builder_add(&builder, "{sv}", "runtime_generation", g_variant_new_uint64(generation));
    g_variant_builder_add(&builder, "{sv}", "operation_id_watermark",
                          g_variant_new_uint64(operation_id_watermark));
    g_variant_builder_add(&builder, "{sv}", "runtime_events",
                          g_variant_new_strv((const char* const*)runtime_events, -1));
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static gboolean queue_packet(Runtime* runtime, GnoblinRuntimePacketType type, guint64 request_id,
                             GVariant* payload, GError** error);
static gboolean dispatch_parent_event(Runtime* runtime, const char* event, GVariant* payload,
                                      GError** error);
static void dispatch_reload_event(Runtime* runtime, const char* event, const char* path,
                                  guint64 revision, const char* failure_message);
static gboolean handle_runtime_packet(Runtime* runtime, GnoblinRuntimePacket* packet,
                                      GError** error);
static void runtime_schedule_deferred_packets(Runtime* runtime);
static void runtime_schedule_deferred_callbacks(gpointer user_data);

static gboolean is_pending_logout(Runtime* runtime, guint64 request_id) {
    return runtime->pending_logout_ids &&
           g_hash_table_contains(runtime->pending_logout_ids, &request_id);
}

static gboolean send_initial_autostart(int fd, GVariant* document, GVariant* hello,
                                       GError** error) {
    g_autoptr(GVariant) entries =
        document ? g_variant_lookup_value(document, "autostart", G_VARIANT_TYPE("av")) : NULL;
    if (!entries)
        entries = g_variant_ref_sink(g_variant_new_array(G_VARIANT_TYPE_VARIANT, NULL, 0));
    g_autoptr(GVariant) environment =
        g_variant_lookup_value(hello, "environment", G_VARIANT_TYPE("a{ss}"));
    if (!environment) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Mutter HELLO did not provide the session environment");
        return FALSE;
    }
    GVariantBuilder payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload, "{sv}", "entries", entries);
    g_variant_builder_add(&payload, "{sv}", "environment", environment);
    g_autoptr(GVariant) packet_payload = g_variant_ref_sink(g_variant_builder_end(&payload));
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    if (!gnoblin_runtime_writer_queue(writer, GNOBLIN_RUNTIME_PACKET_HOST_AUTOSTART, 0,
                                      packet_payload, error))
        return FALSE;
    return gnoblin_runtime_writer_flush(writer, fd, error);
}

static void remember_pending_logout(Runtime* runtime, guint64 request_id) {
    guint64* key = g_new(guint64, 1);
    *key = request_id;
    g_hash_table_add(runtime->pending_logout_ids, key);
}

static gboolean send_config(Runtime* runtime, GVariant* document, guint64 revision,
                            guint64 generation, guint64 transaction_id, GError** error) {
    g_autoptr(GVariant) normalized = gnoblin_native_input_config_normalize(document);
    if (!normalized) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "could not normalize input configuration for Mutter");
        return FALSE;
    }
    g_autoptr(GVariant) payload = config_packet_payload(normalized, revision, generation);
    return queue_packet(runtime, GNOBLIN_RUNTIME_PACKET_CONFIG, transaction_id, payload, error);
}

static void runtime_reload_free(RuntimeReload* reload) {
    if (!reload)
        return;
    if (reload->timeout_id)
        g_source_remove(reload->timeout_id);
    g_free(reload->path);
    g_clear_pointer(&reload->document, g_variant_unref);
    g_free(reload);
}

static void runtime_deferred_packet_free(gpointer data) {
    RuntimeDeferredPacket* packet = data;
    if (!packet)
        return;
    g_clear_pointer(&packet->payload, g_variant_unref);
    g_free(packet);
}

static gboolean send_api_response(Runtime* runtime, guint64 request_id, GVariant* result,
                                  const char* message, GError** error) {
    GVariantBuilder response;
    g_variant_builder_init(&response, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&response, "{sv}", "ok", g_variant_new_boolean(result != NULL));
    if (result)
        g_variant_builder_add(&response, "{sv}", "result", result);
    else
        g_variant_builder_add(
            &response, "{sv}", "error",
            g_variant_new_string(message ? message : "runtime API request failed"));
    g_autoptr(GVariant) packet = g_variant_ref_sink(g_variant_builder_end(&response));
    return queue_packet(runtime, GNOBLIN_RUNTIME_PACKET_API_RESPONSE, request_id, packet, error);
}

static void runtime_reload_finish(Runtime* runtime, gboolean commit, const char* message) {
    RuntimeReload* reload = runtime ? runtime->pending_reload : NULL;
    if (!reload)
        return;
    runtime->pending_reload = NULL;
    gnoblin_config_finish_deferred_load(commit);
    if (commit) {
        g_autoptr(GError) portal_error = NULL;
        if (!gnoblin_config_sync_portal_selection(reload->document, NULL, &portal_error))
            g_warning("gnoblin: could not update portal selection from Lua config: %s",
                      portal_error ? portal_error->message : "unknown error");
        GVariantBuilder builder;
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&builder, "{sv}", "ok", g_variant_new_boolean(TRUE));
        g_variant_builder_add(&builder, "{sv}", "action", g_variant_new_string("config reload"));
        g_variant_builder_add(&builder, "{sv}", "settings_revision",
                              g_variant_new_uint64(reload->revision));
        g_variant_builder_add(&builder, "{sv}", "runtime_generation",
                              g_variant_new_uint64(reload->generation));
        g_autoptr(GVariant) result = g_variant_ref_sink(g_variant_builder_end(&builder));
        dispatch_reload_event(runtime, "gnoblin.config.reloaded", reload->path, reload->revision,
                              NULL);
        g_autoptr(GError) error = NULL;
        if (!send_api_response(runtime, reload->request_id, result, NULL, &error))
            runtime_fail(runtime, error ? error->message : "could not reply to config reload");
    } else {
        dispatch_reload_event(runtime, "gnoblin.config.reload-failed", reload->path, 0, message);
        g_autoptr(GError) error = NULL;
        if (!send_api_response(runtime, reload->request_id, NULL, message, &error))
            runtime_fail(runtime, error ? error->message : "could not reply to config reload");
    }
    runtime_reload_free(reload);
    runtime_schedule_deferred_packets(runtime);
}

static gboolean runtime_reload_timeout(gpointer user_data) {
    Runtime* runtime = user_data;
    if (runtime->pending_reload && runtime->pending_reload->sent) {
        runtime->pending_reload->timeout_id = 0;
        runtime_fail(runtime, "Mutter did not acknowledge the configuration transaction");
    } else {
        if (runtime->pending_reload)
            runtime->pending_reload->timeout_id = 0;
        runtime_reload_finish(runtime, FALSE,
                              "timed out waiting for earlier runtime operations to finish");
    }
    return G_SOURCE_REMOVE;
}

static gboolean runtime_reload_progress(gpointer user_data) {
    Runtime* runtime = user_data;
    runtime->reload_progress_id = 0;
    RuntimeReload* reload = runtime->pending_reload;
    if (!reload || runtime->failed)
        return G_SOURCE_REMOVE;
    if (g_hash_table_size(runtime->pending_operation_ids) != 0 ||
        !g_queue_is_empty(&runtime->policy_events) ||
        gnoblin_config_runtime_pending_operations() != 0)
        return G_SOURCE_REMOVE;
    g_autoptr(GVariant) active_document = gnoblin_config_current_document();
    reload->revision = gnoblin_config_settings_revision();
    if (!active_document || !g_variant_equal(active_document, reload->document))
        reload->revision++;
    g_autoptr(GError) error = NULL;
    if (!send_config(runtime, reload->document, reload->revision, reload->generation,
                     reload->request_id, &error)) {
        runtime_reload_finish(runtime, FALSE,
                              error ? error->message : "could not publish candidate configuration");
        return G_SOURCE_REMOVE;
    }
    reload->sent = TRUE;
    return G_SOURCE_REMOVE;
}

static void runtime_schedule_reload_progress(Runtime* runtime) {
    if (runtime && runtime->pending_reload && !runtime->reload_progress_id)
        runtime->reload_progress_id =
            g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, runtime_reload_progress, runtime, NULL);
}

static gboolean runtime_write_ready(gint fd, GIOCondition condition, gpointer user_data) {
    Runtime* runtime = user_data;
    if (condition & (G_IO_HUP | G_IO_ERR | G_IO_NVAL)) {
        runtime->write_watch_id = 0;
        runtime_fail(runtime, "compositor closed the private runtime channel");
        return G_SOURCE_REMOVE;
    }
    g_autoptr(GError) error = NULL;
    if (!gnoblin_runtime_writer_flush(runtime->writer, fd, &error)) {
        if (error && g_error_matches(error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK))
            return G_SOURCE_CONTINUE;
        runtime->write_watch_id = 0;
        runtime_fail(runtime, error ? error->message : "could not write compositor packet");
        return G_SOURCE_REMOVE;
    }
    runtime->write_watch_id = 0;
    return G_SOURCE_REMOVE;
}

static gboolean queue_packet(Runtime* runtime, GnoblinRuntimePacketType type, guint64 request_id,
                             GVariant* payload, GError** error) {
    if (!gnoblin_runtime_writer_queue(runtime->writer, type, request_id, payload, error))
        return FALSE;
    g_autoptr(GError) flush_error = NULL;
    if (!gnoblin_runtime_writer_flush(runtime->writer, runtime->channel_fd, &flush_error)) {
        if (g_error_matches(flush_error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK)) {
            if (!runtime->write_watch_id)
                runtime->write_watch_id =
                    g_unix_fd_add(runtime->channel_fd, G_IO_OUT | G_IO_HUP | G_IO_ERR | G_IO_NVAL,
                                  runtime_write_ready, runtime);
            return TRUE;
        }
        g_propagate_error(error, g_steal_pointer(&flush_error));
        return FALSE;
    }
    return TRUE;
}

/* The guardian treats STARTED as proof that Mutter has received the initial
 * CONFIG/RESUME packet. queue_packet() may return with bytes still queued when
 * the socket is full, so finish that first packet before publishing the
 * status. Otherwise a supervisor killed in that gap could make the guardian
 * restart without suspending Mutter. */
static gboolean flush_startup_packet(Runtime* runtime, GError** error) {
    gint64 deadline = g_get_monotonic_time() + STARTUP_TIMEOUT_MS * 1000;
    for (;;) {
        g_autoptr(GError) flush_error = NULL;
        if (gnoblin_runtime_writer_flush(runtime->writer, runtime->channel_fd, &flush_error))
            break;
        if (!g_error_matches(flush_error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK)) {
            g_propagate_error(error, g_steal_pointer(&flush_error));
            return FALSE;
        }

        gint64 remaining = deadline - g_get_monotonic_time();
        if (remaining <= 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_TIMED_OUT,
                                "Mutter did not accept the initial runtime packet");
            return FALSE;
        }
        struct pollfd pfd = {.fd = runtime->channel_fd, .events = POLLOUT};
        int timeout = (int)MIN((remaining + 999) / 1000, 100);
        int result;
        do {
            result = poll(&pfd, 1, timeout);
        } while (result < 0 && errno == EINTR);
        if (result < 0 || (result > 0 && (pfd.revents & (POLLERR | POLLHUP | POLLNVAL)))) {
            g_set_error(error, G_IO_ERROR,
                        result < 0 ? g_io_error_from_errno(errno) : G_IO_ERROR_CLOSED,
                        "Mutter closed the runtime channel during startup: %s",
                        result < 0 ? g_strerror(errno) : "peer closed the channel");
            return FALSE;
        }
    }
    if (runtime->write_watch_id) {
        g_source_remove(runtime->write_watch_id);
        runtime->write_watch_id = 0;
    }
    return TRUE;
}

static gboolean send_pending_operations(Runtime* runtime, GError** error) {
    g_autoptr(GVariant) operations = gnoblin_config_drain_runtime_operations();
    if (!operations || !g_variant_is_of_type(operations, G_VARIANT_TYPE("aa{sv}"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Lua runtime returned an invalid operation batch");
        return FALSE;
    }
    GVariantIter iter;
    GVariant* operation;
    g_variant_iter_init(&iter, operations);
    while ((operation = g_variant_iter_next_value(&iter))) {
        g_autoptr(GVariant) owned_operation = operation;
        gint64 request_id = 0;
        const char* method = NULL;
        g_autoptr(GVariant) arguments =
            g_variant_lookup_value(operation, "arguments", G_VARIANT_TYPE_VARDICT);
        if (!g_variant_lookup(operation, "request_id", "x", &request_id) || request_id <= 0 ||
            !g_variant_lookup(operation, "method", "&s", &method) || !method || !*method ||
            !arguments) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "Lua runtime emitted a malformed operation");
            return FALSE;
        }
        gboolean needs_focus_context =
            g_str_equal(method, "window.focus") || g_str_equal(method, "window.begin_move") ||
            g_str_equal(method, "window.begin_resize") ||
            g_str_equal(method, "input.text_target") || g_str_equal(method, "window.snap_context");
        g_autoptr(GVariant) packet = NULL;
        if (needs_focus_context) {
            guint64 handle = 0;
            guint64 runtime_generation = 0;
            guint64 native_generation = 0;
            gint64 expires_at_us = 0;
            if (!gnoblin_config_take_focus_context(request_id, &handle, &runtime_generation,
                                                   &native_generation, &expires_at_us) ||
                !handle || !native_generation || expires_at_us <= g_get_monotonic_time()) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                    "Lua focus operation lost its one-use native context");
                return FALSE;
            }
            if (runtime_generation != gnoblin_config_runtime_generation()) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                    "Lua focus operation belongs to an old runtime generation");
                return FALSE;
            }
            GVariantBuilder operation_builder;
            GVariantIter fields;
            const char* field_name;
            GVariant* field_value;
            g_variant_builder_init(&operation_builder, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&fields, operation);
            while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
                g_autoptr(GVariant) value = field_value;
                g_variant_builder_add(&operation_builder, "{sv}", field_name, value);
            }
            g_variant_builder_add(&operation_builder, "{sv}", "focus_context_handle",
                                  g_variant_new_uint64(handle));
            g_variant_builder_add(&operation_builder, "{sv}", "focus_context_generation",
                                  g_variant_new_uint64(native_generation));
            g_variant_builder_add(&operation_builder, "{sv}", "focus_context_expires_at_us",
                                  g_variant_new_int64(expires_at_us));
            if (g_str_equal(method, "window.snap_context"))
                g_variant_builder_add(&operation_builder, "{sv}", "runtime_generation",
                                      g_variant_new_uint64(gnoblin_config_runtime_generation()));
            packet = g_variant_ref_sink(g_variant_builder_end(&operation_builder));
        } else if (g_str_equal(method, "window.thumbnail") && runtime->current_client_id) {
            GVariantBuilder operation_builder;
            GVariantIter fields;
            const char* field_name;
            GVariant* field_value;
            g_variant_builder_init(&operation_builder, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&fields, operation);
            while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
                g_autoptr(GVariant) value = field_value;
                g_variant_builder_add(&operation_builder, "{sv}", field_name, value);
            }
            g_variant_builder_add(&operation_builder, "{sv}", "client_id",
                                  g_variant_new_uint64(runtime->current_client_id));
            packet = g_variant_ref_sink(g_variant_builder_end(&operation_builder));
        } else if (g_str_equal(method, "window.snap.offer")) {
            GVariantBuilder operation_builder;
            GVariantIter fields;
            const char* field_name;
            GVariant* field_value;
            g_variant_builder_init(&operation_builder, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&fields, operation);
            while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
                g_autoptr(GVariant) value = field_value;
                g_variant_builder_add(&operation_builder, "{sv}", field_name, value);
            }
            g_variant_builder_add(&operation_builder, "{sv}", "runtime_generation",
                                  g_variant_new_uint64(gnoblin_config_runtime_generation()));
            packet = g_variant_ref_sink(g_variant_builder_end(&operation_builder));
        } else if (g_str_equal(method, "window.snap")) {
            GVariantBuilder operation_builder;
            GVariantIter fields;
            const char* field_name;
            GVariant* field_value;
            g_variant_builder_init(&operation_builder, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&fields, operation);
            while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
                g_autoptr(GVariant) value = field_value;
                g_variant_builder_add(&operation_builder, "{sv}", field_name, value);
            }
            g_variant_builder_add(&operation_builder, "{sv}", "runtime_generation",
                                  g_variant_new_uint64(gnoblin_config_runtime_generation()));
            packet = g_variant_ref_sink(g_variant_builder_end(&operation_builder));
        } else {
            packet = g_variant_ref(operation);
        }
        gboolean logout_operation = g_str_equal(method, "session.logout");
        if (logout_operation && is_pending_logout(runtime, (guint64)request_id)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "Lua runtime reused a pending session logout request ID");
            return FALSE;
        }
        if (g_hash_table_contains(runtime->pending_operation_ids,
                                  &(guint64){(guint64)request_id})) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "Lua runtime reused a pending operation request ID");
            return FALSE;
        }
        if (logout_operation)
            remember_pending_logout(runtime, (guint64)request_id);
        if (!queue_packet(runtime, GNOBLIN_RUNTIME_PACKET_OPERATION, (guint64)request_id, packet,
                          error)) {
            if (logout_operation)
                g_hash_table_remove(runtime->pending_logout_ids, &(guint64){(guint64)request_id});
            return FALSE;
        }
        guint64* operation_key = g_new(guint64, 1);
        *operation_key = (guint64)request_id;
        g_hash_table_add(runtime->pending_operation_ids, operation_key);
    }
    return TRUE;
}

static gboolean commit_event_result(Runtime* runtime, GVariant* document, GError** error) {
    if (!document)
        return TRUE;
    gnoblin_config_finish_event(TRUE);
    g_autoptr(GVariant) committed = gnoblin_config_current_document();
    if (!committed) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            "Lua event did not produce a committed configuration");
        return FALSE;
    }
    if (!send_config(runtime, committed, gnoblin_config_settings_revision(),
                     gnoblin_config_runtime_generation(), 0, error))
        return FALSE;
    return TRUE;
}

static gboolean handle_event(Runtime* runtime, GVariant* payload, GError** error) {
    const char* event = NULL;
    g_autoptr(GVariant) event_payload =
        g_variant_lookup_value(payload, "payload", G_VARIANT_TYPE_VARDICT);
    if (!g_variant_lookup(payload, "event", "&s", &event) || !event || !*event || !event_payload) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "compositor sent an invalid runtime event");
        return FALSE;
    }

    g_autoptr(GVariant) document = NULL;
    gboolean trusted_binding = g_str_equal(event, "gnoblin.shortcut.binding-activated");
    gboolean trusted_shortcut = g_str_equal(event, "gnoblin.shortcut.activated");
    gboolean session_key = g_str_equal(event, "gnoblin.shortcut.session.key");
    if (trusted_binding || trusted_shortcut || session_key) {
        gboolean first = FALSE;
        if (trusted_binding && (!g_variant_lookup(event_payload, "first", "b", &first) || !first)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "only a first binding activation may carry focus context");
            return FALSE;
        }
        guint64 handle = 0;
        guint64 native_generation = 0;
        gint64 expires_at_us = 0;
        gboolean has_context =
            g_variant_lookup(payload, "focus_context_handle", "t", &handle) && handle &&
            g_variant_lookup(payload, "focus_context_generation", "t", &native_generation) &&
            native_generation &&
            g_variant_lookup(payload, "focus_context_expires_at_us", "x", &expires_at_us) &&
            expires_at_us > g_get_monotonic_time();
        g_autoptr(GVariant) context_handle =
            g_variant_lookup_value(payload, "focus_context_handle", NULL);
        g_autoptr(GVariant) context_generation =
            g_variant_lookup_value(payload, "focus_context_generation", NULL);
        g_autoptr(GVariant) context_expiry =
            g_variant_lookup_value(payload, "focus_context_expires_at_us", NULL);
        gboolean has_any_context = context_handle || context_generation || context_expiry;
        if (has_context) {
            document = gnoblin_config_dispatch_shortcut_event(
                event, event_payload, handle, native_generation, expires_at_us, error);
        } else if ((trusted_binding || session_key) && !has_any_context) {
            /* These events still reach Lua if optional focus authority is unavailable. */
            document = gnoblin_config_dispatch_event(event, event_payload, error);
        } else {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "shortcut event has no live native focus context");
        }
    } else {
        document = gnoblin_config_dispatch_event(event, event_payload, error);
    }
    if (error && *error) {
        gnoblin_config_finish_event(FALSE);
        return FALSE;
    }
    if (!commit_event_result(runtime, document, error))
        return FALSE;
    return send_pending_operations(runtime, error);
}

static gboolean handle_completion(Runtime* runtime, guint64 packet_id, GVariant* payload,
                                  GError** error) {
    gint64 operation_id = 0;
    if (!g_variant_lookup(payload, "operation_id", "x", &operation_id) || operation_id <= 0 ||
        (guint64)operation_id != packet_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "compositor completion request ID does not match its frame");
        return FALSE;
    }

    gboolean logout_operation = is_pending_logout(runtime, packet_id);
    const char* method = NULL;
    gboolean succeeded = FALSE;
    gboolean logout_accepted = FALSE;
    g_autoptr(GVariant) operation_result =
        g_variant_lookup_value(payload, "value", G_VARIANT_TYPE_VARDICT);
    if (operation_result)
        g_variant_lookup(operation_result, "accepted", "b", &logout_accepted);
    gboolean matching_success = logout_operation &&
                                g_variant_lookup(payload, "method", "&s", &method) &&
                                g_str_equal(method, "session.logout") &&
                                g_variant_lookup(payload, "ok", "b", &succeeded) && succeeded;
    matching_success = matching_success && operation_result && logout_accepted;

    g_autoptr(GVariant) document =
        gnoblin_config_dispatch_event("gnoblin.operation.completed", payload, error);
    if (error && *error) {
        gnoblin_config_finish_event(FALSE);
        return FALSE;
    }
    if (!commit_event_result(runtime, document, error))
        return FALSE;
    if (!send_pending_operations(runtime, error))
        return FALSE;

    g_hash_table_remove(runtime->pending_operation_ids, &packet_id);
    runtime_schedule_reload_progress(runtime);

    if (logout_operation) {
        g_hash_table_remove(runtime->pending_logout_ids, &packet_id);
        if (matching_success) {
            runtime->logout_requested = TRUE;
            runtime->exit_status = EXIT_SUCCESS;
            g_main_loop_quit(runtime->loop);
        }
    }
    return TRUE;
}

static gboolean dispatch_parent_event(Runtime* runtime, const char* event, GVariant* payload,
                                      GError** error) {
    g_autoptr(GVariant) document = gnoblin_config_dispatch_event(event, payload, error);
    if (error && *error) {
        gnoblin_config_finish_event(FALSE);
        return FALSE;
    }
    if (!commit_event_result(runtime, document, error))
        return FALSE;
    return send_pending_operations(runtime, error);
}

static gboolean dispatch_deferred_callbacks(gpointer user_data) {
    Runtime* runtime = user_data;
    runtime->deferred_callback_idle_id = 0;
    if (runtime->failed)
        return G_SOURCE_REMOVE;

    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) document = gnoblin_config_dispatch_deferred_callbacks(&error);
    if (error) {
        gnoblin_config_finish_event(FALSE);
        g_warning("gnoblin: deferred Lua operation callback failed: %s", error->message);
        g_clear_error(&error);
    } else if (!commit_event_result(runtime, document, &error)) {
        runtime_fail(runtime,
                     error ? error->message : "could not commit deferred Lua operation callback");
        return G_SOURCE_REMOVE;
    }
    if (!send_pending_operations(runtime, &error)) {
        runtime_fail(runtime,
                     error ? error->message : "could not send deferred Lua callback operations");
        return G_SOURCE_REMOVE;
    }
    runtime_schedule_reload_progress(runtime);
    return G_SOURCE_REMOVE;
}

static void runtime_schedule_deferred_callbacks(gpointer user_data) {
    Runtime* runtime = user_data;
    if (!runtime || runtime->failed || runtime->deferred_callback_idle_id)
        return;
    runtime->deferred_callback_idle_id =
        g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, dispatch_deferred_callbacks, runtime, NULL);
}

static GVariant* policy_event_payload(Runtime* runtime, GVariant* policy, guint64 revision) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "policy", policy);
    g_variant_builder_add(&builder, "{sv}", "revision", g_variant_new_int64((gint64)revision));
    g_variant_builder_add(&builder, "{sv}", "sequence",
                          g_variant_new_int64((gint64)++runtime->event_sequence));
    g_variant_builder_add(&builder, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static void runtime_policy_event_free(gpointer data) {
    RuntimePolicyEvent* event = data;
    if (!event)
        return;
    g_clear_pointer(&event->payload, g_variant_unref);
    g_free(event);
}

static gboolean dispatch_commit_event(Runtime* runtime, const char* event, GVariant* payload) {
    g_autoptr(GError) error = NULL;
    if (!dispatch_parent_event(runtime, event, payload, &error)) {
        g_warning("gnoblin: could not dispatch %s: %s", event,
                  error ? error->message : "unknown runtime error");
        return FALSE;
    }
    return TRUE;
}

static gboolean dispatch_policy_events(gpointer user_data) {
    Runtime* runtime = user_data;
    runtime->policy_event_idle_id = 0;
    while (!g_queue_is_empty(&runtime->policy_events)) {
        RuntimePolicyEvent* event = g_queue_pop_head(&runtime->policy_events);
        dispatch_commit_event(runtime, event->event, event->payload);
        runtime_policy_event_free(event);
    }
    runtime_schedule_reload_progress(runtime);
    return G_SOURCE_REMOVE;
}

static void queue_policy_event(Runtime* runtime, const char* event, GVariant* policy,
                               guint64 revision) {
    if (!runtime || runtime->failed || !policy)
        return;
    RuntimePolicyEvent* queued = g_new0(RuntimePolicyEvent, 1);
    queued->event = event;
    queued->payload = policy_event_payload(runtime, policy, revision);
    g_queue_push_tail(&runtime->policy_events, queued);
    if (!runtime->policy_event_idle_id)
        runtime->policy_event_idle_id =
            g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, dispatch_policy_events, runtime, NULL);
}

static void focus_policy_committed(GVariant* policy, guint64 revision, gpointer user_data) {
    queue_policy_event(user_data, "gnoblin.focus.policy-changed", policy, revision);
}

static void permission_policy_committed(GVariant* policy, guint64 revision, gpointer user_data) {
    queue_policy_event(user_data, "gnoblin.permission.changed", policy, revision);
}

static void dispatch_reload_event(Runtime* runtime, const char* event, const char* path,
                                  guint64 revision, const char* failure_message) {
    GVariantBuilder payload_builder;
    g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload_builder, "{sv}", "path", g_variant_new_string(path));
    g_variant_builder_add(&payload_builder, "{sv}", "sequence",
                          g_variant_new_int64((gint64)++runtime->event_sequence));
    g_variant_builder_add(&payload_builder, "{sv}", "time",
                          g_variant_new_int64(g_get_monotonic_time()));
    if (g_str_equal(event, "gnoblin.config.reloaded"))
        g_variant_builder_add(&payload_builder, "{sv}", "revision", g_variant_new_uint64(revision));
    else
        g_variant_builder_add(&payload_builder, "{sv}", "error",
                              g_variant_new_string(failure_message
                                                       ? failure_message
                                                       : "configuration reload failed"));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
    dispatch_commit_event(runtime, event, payload);
}

static gboolean handle_state(Runtime* runtime, GVariant* payload, GError** error) {
    guint32 version = 0;
    guint64 revision = 0;
    gboolean available = FALSE;
    const char* name = NULL;
    g_autoptr(GVariant) snapshot = NULL;
    if (!g_variant_lookup(payload, "state_version", "u", &version) || version != 1 ||
        !g_variant_lookup(payload, "revision", "t", &revision) ||
        !g_variant_lookup(payload, "available", "b", &available) ||
        !g_variant_lookup(payload, "name", "&s", &name) || !name || !*name) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "compositor sent an invalid state snapshot");
        return FALSE;
    }
    if (available) {
        snapshot = g_variant_lookup_value(payload, "snapshot", G_VARIANT_TYPE_VARDICT);
        if (!snapshot) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "compositor state snapshot is missing a valid a{sv} value");
            return FALSE;
        }
    } else {
        g_autoptr(GVariant) unexpected_snapshot = g_variant_lookup_value(payload, "snapshot", NULL);
        if (unexpected_snapshot) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "unavailable compositor state snapshot must not include a value");
            return FALSE;
        }
    }
    guint64* previous = g_hash_table_lookup(runtime->state_revisions, name);
    if (previous && revision < *previous) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                    "compositor state snapshot '%s' moved backwards in revision", name);
        return FALSE;
    }
    void (*update)(GVariant*, guint64) = NULL;
    if (g_str_equal(name, "windows"))
        update = gnoblin_config_update_window_snapshot;
    else if (g_str_equal(name, "workspaces"))
        update = gnoblin_config_update_workspace_snapshot;
    else if (g_str_equal(name, "monitors"))
        update = gnoblin_config_update_monitor_snapshot;
    else if (g_str_equal(name, "monitor-privacy-screen"))
        update = gnoblin_config_update_monitor_privacy_screen_snapshot;
    else if (g_str_equal(name, "layers"))
        update = gnoblin_config_update_layer_snapshot;
    else if (g_str_equal(name, "capabilities"))
        update = gnoblin_config_update_capability_snapshot;
    else if (g_str_equal(name, "input-devices"))
        update = gnoblin_config_update_input_device_snapshot;
    else if (g_str_equal(name, "input-sources"))
        update = gnoblin_config_update_input_source_snapshot;
    else if (g_str_equal(name, "input-orientation-lock"))
        update = gnoblin_config_update_orientation_lock_snapshot;
    else if (g_str_equal(name, "appearance"))
        update = gnoblin_config_update_appearance_snapshot;
    else if (g_str_equal(name, "shortcuts"))
        update = gnoblin_config_update_shortcut_snapshot;
    else if (g_str_equal(name, "launches"))
        update = gnoblin_config_update_launch_snapshot;
    else if (g_str_equal(name, "portal-grants"))
        update = gnoblin_config_update_portal_grant_snapshot;
    else if (g_str_equal(name, "privacy"))
        update = gnoblin_config_update_privacy_snapshot;
    else if (g_str_equal(name, "session-activity"))
        update = gnoblin_config_update_session_activity_snapshot;
    else if (g_str_equal(name, "session-lock"))
        update = gnoblin_config_update_session_lock_snapshot;
    else if (g_str_equal(name, "animations"))
        update = gnoblin_config_update_animation_snapshot;
    if (!update) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                    "compositor state snapshot '%s' is unsupported", name);
        return FALSE;
    }
    update(snapshot, revision);
    guint64* saved_revision = g_new(guint64, 1);
    *saved_revision = revision;
    g_hash_table_replace(runtime->state_revisions, g_strdup(name), saved_revision);
    return TRUE;
}

static gboolean handle_api_request(Runtime* runtime, guint64 request_id, GVariant* payload,
                                   GError** error) {
    const char* kind = NULL;
    const char* method = NULL;
    g_autoptr(GVariant) arguments =
        g_variant_lookup_value(payload, "arguments", G_VARIANT_TYPE_VARDICT);
    guint64 client_id = 0;
    if (request_id == 0 || !g_variant_lookup(payload, "kind", "&s", &kind) ||
        !g_variant_lookup(payload, "method", "&s", &method) || !*method || !arguments ||
        (!g_str_equal(kind, "read") && !g_str_equal(kind, "call") &&
         !g_str_equal(kind, "reload"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "compositor sent an invalid runtime API request");
        return FALSE;
    }
    if (g_str_equal(kind, "call"))
        g_variant_lookup(payload, "client_id", "t", &client_id);
    runtime->current_client_id = client_id;

    g_autoptr(GError) api_error = NULL;
    g_autoptr(GVariant) result = NULL;
    if (g_str_equal(kind, "read")) {
        result = gnoblin_config_read_api(method, arguments, &api_error);
    } else if (g_str_equal(kind, "call")) {
        result = gnoblin_config_call_api(method, arguments, &api_error);
    } else if (!g_str_equal(method, "runtime.reload_config") ||
               g_variant_n_children(arguments) != 0) {
        g_set_error_literal(&api_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "invalid runtime config reload request");
    } else {
        if (runtime->pending_reload) {
            g_set_error_literal(&api_error, G_IO_ERROR, G_IO_ERROR_BUSY,
                                "a configuration reload is already in progress");
        } else {
            g_autofree char* path = gnoblin_config_path();
            g_autoptr(GVariant) candidate =
                gnoblin_config_load_runtime(path, NULL, NULL, &api_error);
            if (candidate) {
                if (!gnoblin_config_defer_load_commit()) {
                    gnoblin_config_finish_load(FALSE);
                    g_set_error_literal(&api_error, G_IO_ERROR, G_IO_ERROR_BUSY,
                                        "could not stage configuration reload");
                    dispatch_reload_event(runtime, "gnoblin.config.reload-failed", path, 0,
                                          api_error->message);
                } else {
                    RuntimeReload* reload = g_new0(RuntimeReload, 1);
                    reload->request_id = request_id;
                    reload->generation = gnoblin_config_deferred_runtime_generation();
                    reload->path = g_strdup(path);
                    reload->document = g_variant_ref(candidate);
                    reload->timeout_id = g_timeout_add_seconds(15, runtime_reload_timeout, runtime);
                    runtime->pending_reload = reload;
                    runtime_schedule_reload_progress(runtime);
                    runtime->current_client_id = 0;
                    return TRUE;
                }
            } else {
                gnoblin_config_finish_load(FALSE);
                dispatch_reload_event(runtime, "gnoblin.config.reload-failed", path, 0,
                                      api_error ? api_error->message : NULL);
            }
        }
    }
    if (!send_api_response(runtime, request_id, result, api_error ? api_error->message : NULL,
                           error)) {
        runtime->current_client_id = 0;
        return FALSE;
    }
    gboolean sent = !g_str_equal(kind, "call") || send_pending_operations(runtime, error);
    runtime->current_client_id = 0;
    return sent;
}

static gboolean startup_timeout(gpointer user_data) {
    Runtime* runtime = user_data;
    if (!runtime->ready)
        runtime_fail(runtime, "compositor did not complete the private runtime handshake");
    runtime->startup_timeout_id = 0;
    return G_SOURCE_REMOVE;
}

static gboolean runtime_signal_received(gpointer user_data) {
    RuntimeSignal* signal = user_data;
    Runtime* runtime = signal->runtime;
    runtime->signal_exit_status = 128 + signal->signal_number;
    runtime_fail(runtime, signal->signal_number == SIGINT
                              ? "received SIGINT; stopping compositor"
                              : "received SIGTERM; stopping compositor");
    return G_SOURCE_REMOVE;
}

static gboolean handle_runtime_packet(Runtime* runtime, GnoblinRuntimePacket* packet,
                                      GError** error) {
    gboolean handled = FALSE;
    if (!runtime->ready) {
        const char* role = NULL;
        guint32 document_version = 0;
        handled = packet->type == GNOBLIN_RUNTIME_PACKET_HELLO && packet->request_id == 0 &&
                  g_variant_lookup(packet->payload, "role", "&s", &role) &&
                  g_strcmp0(role, "compositor") == 0 &&
                  g_variant_lookup(packet->payload, "document_version", "u", &document_version) &&
                  document_version == 1;
        if (!handled)
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                                "compositor runtime handshake is incompatible");
        else {
            if (runtime->send_initial_autostart &&
                !send_initial_autostart(runtime->host_control_fd, runtime->initial_document,
                                        packet->payload, error))
                return FALSE;
            if (runtime->host_control_fd >= 0) {
                close(runtime->host_control_fd);
                runtime->host_control_fd = -1;
            }
            runtime->ready = TRUE;
            if (runtime->guardian_status_fd >= 0)
                (void)guardian_send_status(runtime->guardian_status_fd, GUARDIAN_STATUS_READY);
            /* The host uses this one-byte notification to distinguish a
             * startup failure from a worker crash after the first handshake. */
            if (runtime->ready_fd >= 0) {
                const guint8 ready = 1;
                (void)write(runtime->ready_fd, &ready, sizeof ready);
                close(runtime->ready_fd);
                runtime->ready_fd = -1;
            }
            if (runtime->startup_timeout_id) {
                g_source_remove(runtime->startup_timeout_id);
                runtime->startup_timeout_id = 0;
            }
            runtime_schedule_deferred_packets(runtime);
        }
    } else if (packet->type == GNOBLIN_RUNTIME_PACKET_EVENT && packet->request_id == 0) {
        handled = handle_event(runtime, packet->payload, error);
    } else if (packet->type == GNOBLIN_RUNTIME_PACKET_COMPLETION && packet->request_id > 0) {
        handled = handle_completion(runtime, packet->request_id, packet->payload, error);
    } else if (packet->type == GNOBLIN_RUNTIME_PACKET_CONFIG_RESULT && packet->request_id > 0) {
        RuntimeReload* reload = runtime->pending_reload;
        gboolean accepted = FALSE;
        guint64 revision = 0;
        guint64 generation = 0;
        const char* message = NULL;
        if (!reload || !reload->sent || reload->request_id != packet->request_id ||
            !g_variant_lookup(packet->payload, "accepted", "b", &accepted) ||
            !g_variant_lookup(packet->payload, "settings_revision", "t", &revision) ||
            !g_variant_lookup(packet->payload, "runtime_generation", "t", &generation) ||
            revision != reload->revision || generation != reload->generation) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "Mutter sent an invalid configuration transaction result");
        } else {
            g_variant_lookup(packet->payload, "error", "&s", &message);
            runtime_reload_finish(runtime, accepted,
                                  message ? message : "Mutter rejected the configuration");
            handled = TRUE;
        }
    } else if (packet->type == GNOBLIN_RUNTIME_PACKET_API_REQUEST && packet->request_id > 0) {
        handled = handle_api_request(runtime, packet->request_id, packet->payload, error);
    } else if (packet->type == GNOBLIN_RUNTIME_PACKET_STATE && packet->request_id == 0) {
        handled = handle_state(runtime, packet->payload, error);
    } else if (packet->type == GNOBLIN_RUNTIME_PACKET_ERROR) {
        const char* message = NULL;
        gboolean resume_rejected = FALSE;
        g_variant_lookup(packet->payload, "message", "&s", &message);
        g_variant_lookup(packet->payload, "resume_rejected", "b", &resume_rejected);
        runtime->resume_rejected = runtime->resume_worker && resume_rejected;
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "compositor rejected runtime: %s",
                    message ? message : "unspecified error");
    } else {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "unexpected packet from compositor");
    }
    return handled;
}

static gboolean runtime_dispatch_deferred_packets(gpointer user_data) {
    Runtime* runtime = user_data;
    runtime->deferred_packets_idle_id = 0;
    while (!runtime->pending_reload && !g_queue_is_empty(&runtime->deferred_packets) &&
           !runtime->failed) {
        RuntimeDeferredPacket* deferred = g_queue_pop_head(&runtime->deferred_packets);
        GnoblinRuntimePacket packet = {.type = deferred->type,
                                       .request_id = deferred->request_id,
                                       .payload = g_steal_pointer(&deferred->payload)};
        runtime_deferred_packet_free(deferred);
        g_autoptr(GError) error = NULL;
        gboolean handled = handle_runtime_packet(runtime, &packet, &error);
        gnoblin_runtime_packet_clear(&packet);
        if (error || !handled) {
            runtime_fail(runtime,
                         error ? error->message : "could not process deferred compositor packet");
            break;
        }
    }
    if (!runtime->pending_reload && !g_queue_is_empty(&runtime->deferred_packets) &&
        !runtime->failed)
        runtime_schedule_deferred_packets(runtime);
    return G_SOURCE_REMOVE;
}

static void runtime_schedule_deferred_packets(Runtime* runtime) {
    if (runtime && !runtime->pending_reload && !runtime->deferred_packets_idle_id &&
        !g_queue_is_empty(&runtime->deferred_packets))
        runtime->deferred_packets_idle_id = g_idle_add_full(
            G_PRIORITY_DEFAULT_IDLE, runtime_dispatch_deferred_packets, runtime, NULL);
}

static gboolean runtime_channel_ready(gint fd, GIOCondition condition, gpointer user_data) {
    Runtime* runtime = user_data;
    if (condition & (G_IO_HUP | G_IO_ERR | G_IO_NVAL)) {
        runtime_fail(runtime, "compositor closed the private runtime channel");
        return G_SOURCE_REMOVE;
    }
    for (;;) {
        g_autoptr(GError) error = NULL;
        GnoblinRuntimePacket packet = {0};
        gboolean available = FALSE;
        if (!gnoblin_runtime_reader_receive(runtime->reader, fd, &packet, &available, &error)) {
            runtime_fail(runtime, error ? error->message : "could not read compositor packet");
            return G_SOURCE_REMOVE;
        }
        if (!available)
            return G_SOURCE_CONTINUE;
        /* Mutter can publish session activity while finishing startup. Keep
         * those events ordered until HELLO establishes the runtime contract. */
        if (!runtime->ready && packet.type == GNOBLIN_RUNTIME_PACKET_EVENT &&
            packet.request_id == 0) {
            if (g_queue_get_length(&runtime->deferred_packets) >= 256) {
                gnoblin_runtime_packet_clear(&packet);
                runtime_fail(runtime, "too many compositor events arrived before HELLO");
                return G_SOURCE_REMOVE;
            }
            RuntimeDeferredPacket* deferred = g_new0(RuntimeDeferredPacket, 1);
            deferred->type = packet.type;
            deferred->request_id = packet.request_id;
            deferred->payload = g_steal_pointer(&packet.payload);
            g_queue_push_tail(&runtime->deferred_packets, deferred);
            continue;
        }
        if ((runtime->pending_reload || !g_queue_is_empty(&runtime->deferred_packets)) &&
            (packet.type == GNOBLIN_RUNTIME_PACKET_EVENT ||
             packet.type == GNOBLIN_RUNTIME_PACKET_API_REQUEST ||
             packet.type == GNOBLIN_RUNTIME_PACKET_STATE)) {
            if (g_queue_get_length(&runtime->deferred_packets) >= 256) {
                gnoblin_runtime_packet_clear(&packet);
                runtime_fail(runtime, "too many runtime events arrived during config reload");
                return G_SOURCE_REMOVE;
            }
            RuntimeDeferredPacket* deferred = g_new0(RuntimeDeferredPacket, 1);
            deferred->type = packet.type;
            deferred->request_id = packet.request_id;
            deferred->payload = g_steal_pointer(&packet.payload);
            g_queue_push_tail(&runtime->deferred_packets, deferred);
            continue;
        }
        gboolean handled = handle_runtime_packet(runtime, &packet, &error);
        gnoblin_runtime_packet_clear(&packet);
        if (error) {
            runtime_fail(runtime, error->message);
            return G_SOURCE_REMOVE;
        }
        if (runtime->logout_requested)
            return G_SOURCE_REMOVE;
        if (!handled) {
            runtime_fail(runtime, "could not process compositor runtime packet");
            return G_SOURCE_REMOVE;
        }
    }
    return G_SOURCE_CONTINUE;
}

static void terminate_and_reap(Runtime* runtime) {
    if (!runtime || !runtime->compositor_pid)
        return;
    if (runtime->child_watch_id) {
        g_source_remove(runtime->child_watch_id);
        runtime->child_watch_id = 0;
    }
    GPid pid = runtime->compositor_pid;
    runtime->compositor_pid = 0;
    kill(pid, SIGTERM);
    int status = 0;
    gboolean reaped = FALSE;
    for (guint i = 0; i < 200; i++) {
        pid_t result = waitpid(pid, &status, WNOHANG);
        if (result == pid) {
            reaped = TRUE;
            break;
        }
        if (result < 0 && errno == ECHILD) {
            reaped = TRUE;
            break;
        }
        if (result < 0 && errno != EINTR)
            break;
        g_usleep(10000);
    }
    if (!reaped) {
        kill(pid, SIGKILL);
        while (waitpid(pid, &status, 0) < 0 && errno == EINTR) {
        }
    }
    g_spawn_close_pid(pid);
}

/* Reap the compositor when it exits during a blocking host wait. Keeping the
 * PID in Runtime after waitpid() has consumed it risks signalling a reused PID
 * during the normal shutdown path. */
static gboolean host_reap_compositor(Runtime* runtime) {
    if (!runtime)
        return TRUE;
    if (!runtime->compositor_pid)
        return FALSE;

    GPid pid = runtime->compositor_pid;
    gint status = 0;
    pid_t result;
    do {
        result = waitpid(pid, &status, WNOHANG);
    } while (result < 0 && errno == EINTR);

    if (result == 0)
        return FALSE;

    runtime->compositor_pid = 0;
    if (result == pid) {
        runtime->exit_status = WIFEXITED(status)     ? WEXITSTATUS(status)
                               : WIFSIGNALED(status) ? 128 + WTERMSIG(status)
                                                     : EXIT_FAILURE;
    } else {
        runtime->exit_status = EXIT_FAILURE;
    }
    g_spawn_close_pid(pid);
    return TRUE;
}

static GPid spawn_compositor(const char* path, const char* plugin, int parent_fd, int child_fd,
                             gboolean xwayland, gboolean devkit, const char* wayland_display,
                             GError** error) {
    posix_spawn_file_actions_t actions;
    int result = posix_spawn_file_actions_init(&actions);
    if (result != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(result),
                    "could not prepare compositor spawn: %s", g_strerror(result));
        return 0;
    }
    int temporary_fd = -1;
    if (!gnoblin_runtime_spawn_add_channel_actions(&actions, parent_fd, child_fd, RUNTIME_FD,
                                                   &temporary_fd, error)) {
        posix_spawn_file_actions_destroy(&actions);
        return 0;
    }

    g_autofree char* fd_option = g_strdup_printf("--gnoblin-runtime-fd=%d", RUNTIME_FD);
    g_autoptr(GPtrArray) argv = g_ptr_array_new_with_free_func(g_free);
    g_ptr_array_add(argv, g_strdup(path));
    g_ptr_array_add(argv, g_strdup("--wayland"));
    if (devkit) {
        g_ptr_array_add(argv, g_strdup("--virtual-monitor"));
        g_ptr_array_add(argv, g_strdup("1280x720@60"));
        g_ptr_array_add(argv, g_strdup("--devkit"));
        g_ptr_array_add(argv, g_strdup("--wayland-display"));
        g_ptr_array_add(argv, g_strdup(wayland_display));
    }
    g_ptr_array_add(argv, g_strdup("--mutter-plugin"));
    g_ptr_array_add(argv, g_strdup(plugin));
    g_ptr_array_add(argv, g_steal_pointer(&fd_option));
    if (!xwayland)
        g_ptr_array_add(argv, g_strdup("--no-x11"));
    g_ptr_array_add(argv, NULL);

    pid_t pid = 0;
    result = posix_spawn(&pid, path, &actions, NULL, (char**)argv->pdata, environ);
    posix_spawn_file_actions_destroy(&actions);
    if (temporary_fd >= 0)
        close(temporary_fd);
    if (result != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(result),
                    "could not start Gnoblin compositor %s: %s", path, g_strerror(result));
        return 0;
    }
    return (GPid)pid;
}

static void usage(const char* program) {
    g_printerr("Usage: %s [--config PATH] [--xwayland|--no-xwayland] "
               "[--devkit --wayland-display NAME]\n",
               program);
}

static int runtime_worker_main(int argc, char** argv) {
    if (argc >= 2 && g_str_equal(argv[1], "--version")) {
        g_autofree char* ctl = NULL;
        g_autofree char* exe = g_file_read_link("/proc/self/exe", NULL);
        if (exe) {
            g_autofree char* bin_dir = g_path_get_dirname(exe);
            g_autofree char* prefix = g_path_get_dirname(bin_dir);
            ctl = g_build_filename(prefix, "bin/gnoblinctl", NULL);
        }
        if (!ctl)
            return EXIT_FAILURE;
        argv[0] = ctl;
        execv(ctl, argv);
        g_printerr("gnoblin: could not run gnoblinctl: %s\n", g_strerror(errno));
        return EXIT_FAILURE;
    }
    g_autofree char* config_path = NULL;
    const char* wayland_display = NULL;
    gboolean devkit = FALSE;
    gboolean resume_worker = FALSE;
    gboolean send_autostart = FALSE;
    int ready_fd = -1;
    int host_control_fd = -1;
    guint64 seed_settings_revision = 0;
    guint64 seed_runtime_generation = 0;
    guint64 seed_operation_id_watermark = 0;
    g_autoptr(GError) error = NULL;
    for (int i = 1; i < argc; i++) {
        if (g_str_equal(argv[i], "--config") && i + 1 < argc)
            config_path = g_canonicalize_filename(argv[++i], NULL);
        else if (g_str_equal(argv[i], "--devkit"))
            devkit = TRUE;
        else if (g_str_equal(argv[i], "--wayland-display") && i + 1 < argc)
            wayland_display = argv[++i];
        else if (g_str_equal(argv[i], "--internal-runtime-worker"))
            ;
        else if (g_str_equal(argv[i], "--resume-runtime-worker"))
            resume_worker = TRUE;
        else if (g_str_equal(argv[i], "--send-initial-autostart"))
            send_autostart = TRUE;
        else if (g_str_equal(argv[i], "--worker-ready-fd") && i + 1 < argc)
            ready_fd = (int)g_ascii_strtoll(argv[++i], NULL, 10);
        else if (g_str_equal(argv[i], "--worker-host-fd") && i + 1 < argc)
            host_control_fd = (int)g_ascii_strtoll(argv[++i], NULL, 10);
        else if (g_str_equal(argv[i], "--seed-settings-revision") && i + 1 < argc)
            seed_settings_revision = g_ascii_strtoull(argv[++i], NULL, 10);
        else if (g_str_equal(argv[i], "--seed-runtime-generation") && i + 1 < argc)
            seed_runtime_generation = g_ascii_strtoull(argv[++i], NULL, 10);
        else if (g_str_equal(argv[i], "--seed-operation-id-watermark") && i + 1 < argc)
            seed_operation_id_watermark = g_ascii_strtoull(argv[++i], NULL, 10);
        else {
            usage(argv[0]);
            return EXIT_FAILURE;
        }
    }
    if (devkit != (wayland_display != NULL) ||
        (wayland_display &&
         (!g_regex_match_simple("^[A-Za-z0-9_.-]{1,128}$", wayland_display, 0, 0) ||
          g_str_equal(wayland_display, ".") || g_str_equal(wayland_display, "..")))) {
        usage(argv[0]);
        g_printerr("gnoblin: devkit mode needs a valid Wayland display name\n");
        return EXIT_FAILURE;
    }
    send_autostart |= !resume_worker;
    if (ready_fd < 0 || (send_autostart && host_control_fd < 0)) {
        g_printerr("gnoblin: invalid internal runtime worker arguments\n");
        return EXIT_FAILURE;
    }
    if (fcntl(WORKER_GUARDIAN_STATUS_FD, F_GETFD) < 0) {
        g_printerr("gnoblin: runtime worker has no guardian status channel\n");
        close(ready_fd);
        return EXIT_FAILURE;
    }
    if (send_autostart) {
        int type = 0;
        socklen_t type_size = sizeof type;
        if (fcntl(host_control_fd, F_GETFD) < 0 ||
            getsockopt(host_control_fd, SOL_SOCKET, SO_TYPE, &type, &type_size) != 0 ||
            type != SOCK_SEQPACKET) {
            g_printerr("gnoblin: invalid inherited supervisor channel\n");
            close(ready_fd);
            return EXIT_FAILURE;
        }
    }
    if (resume_worker &&
        !gnoblin_config_seed_runtime_counters(seed_settings_revision, seed_runtime_generation,
                                              seed_operation_id_watermark, &error)) {
        g_printerr("gnoblin: could not seed replacement runtime counters: %s\n",
                   error ? error->message : "invalid runtime counters");
        close(ready_fd);
        return EXIT_FAILURE;
    }
    if (!config_path)
        config_path = gnoblin_config_path();
    g_setenv("GNOBLIN_CONFIG", config_path, TRUE);

    g_autoptr(GPtrArray) paths = NULL;
    g_autoptr(GPtrArray) directories = NULL;
    g_autoptr(GVariant) document =
        gnoblin_config_load_runtime(config_path, &paths, &directories, &error);
    if (!document) {
        g_printerr("gnoblin: could not load %s: %s\n", config_path,
                   error ? error->message : "invalid configuration");
        gnoblin_config_finish_load(FALSE);
        return EXIT_FAILURE;
    }
    gnoblin_config_finish_load(TRUE);
    guint64 revision = gnoblin_config_settings_revision();

    Runtime runtime = {
        .channel_fd = RUNTIME_FD,
        .ready_fd = ready_fd,
        .host_control_fd = host_control_fd,
        .guardian_status_fd = WORKER_GUARDIAN_STATUS_FD,
        .initial_document = send_autostart ? g_variant_ref(document) : NULL,
        .resume_worker = resume_worker,
        .send_initial_autostart = send_autostart,
        .exit_status = EXIT_FAILURE,
        .writer = gnoblin_runtime_writer_new(),
        .reader = gnoblin_runtime_reader_new(),
        .state_revisions = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free),
        .pending_logout_ids = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL),
        .pending_operation_ids = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL)};
    g_queue_init(&runtime.policy_events);
    g_queue_init(&runtime.deferred_packets);
    gnoblin_config_set_focus_policy_changed_callback(focus_policy_committed, &runtime);
    gnoblin_config_set_permission_policy_changed_callback(permission_policy_committed, &runtime);
    runtime.loop = g_main_loop_new(NULL, FALSE);
    RuntimeSignal sigterm = {.runtime = &runtime, .signal_number = SIGTERM};
    RuntimeSignal sigint = {.runtime = &runtime, .signal_number = SIGINT};
    g_unix_signal_add(SIGTERM, runtime_signal_received, &sigterm);
    g_unix_signal_add(SIGINT, runtime_signal_received, &sigint);
    if (fcntl(RUNTIME_FD, F_GETFD) < 0) {
        gnoblin_runtime_writer_free(runtime.writer);
        gnoblin_runtime_reader_free(runtime.reader);
        gnoblin_config_set_focus_policy_changed_callback(NULL, NULL);
        gnoblin_config_set_permission_policy_changed_callback(NULL, NULL);
        g_hash_table_unref(runtime.state_revisions);
        g_hash_table_unref(runtime.pending_logout_ids);
        g_hash_table_unref(runtime.pending_operation_ids);
        g_main_loop_unref(runtime.loop);
        g_printerr("gnoblin: runtime worker has no inherited channel descriptor\n");
        return EXIT_FAILURE;
    }
    g_autoptr(GVariant) normalized_document =
        resume_worker ? gnoblin_native_input_config_normalize(document) : NULL;
    if (resume_worker && !normalized_document) {
        g_printerr("gnoblin: could not normalize input configuration for Mutter\n");
        close(runtime.ready_fd);
        close(runtime.channel_fd);
        gnoblin_runtime_writer_free(runtime.writer);
        gnoblin_runtime_reader_free(runtime.reader);
        gnoblin_config_set_focus_policy_changed_callback(NULL, NULL);
        gnoblin_config_set_permission_policy_changed_callback(NULL, NULL);
        g_hash_table_unref(runtime.state_revisions);
        g_hash_table_unref(runtime.pending_logout_ids);
        g_hash_table_unref(runtime.pending_operation_ids);
        g_main_loop_unref(runtime.loop);
        return EXIT_FAILURE;
    }
    g_autoptr(GVariant) resume_payload =
        resume_worker ? resume_packet_payload(normalized_document, revision,
                                              gnoblin_config_runtime_generation(),
                                              seed_operation_id_watermark)
                      : NULL;
    gboolean sent_startup_packet =
        resume_worker ? queue_packet(&runtime, GNOBLIN_RUNTIME_PACKET_WORKER_RESUME, 0,
                                     resume_payload, &error)
                      : send_config(&runtime, document, revision,
                                    gnoblin_config_runtime_generation(), 0, &error);
    if (sent_startup_packet)
        sent_startup_packet = flush_startup_packet(&runtime, &error);
    if (!sent_startup_packet) {
        close(runtime.ready_fd);
        close(runtime.channel_fd);
        gnoblin_runtime_writer_free(runtime.writer);
        gnoblin_runtime_reader_free(runtime.reader);
        gnoblin_config_set_focus_policy_changed_callback(NULL, NULL);
        gnoblin_config_set_permission_policy_changed_callback(NULL, NULL);
        g_queue_clear_full(&runtime.policy_events, runtime_policy_event_free);
        g_hash_table_unref(runtime.state_revisions);
        g_hash_table_unref(runtime.pending_logout_ids);
        g_hash_table_unref(runtime.pending_operation_ids);
        g_printerr("gnoblin: could not send initial runtime packet: %s\n",
                   error ? error->message : "channel write failed");
        g_main_loop_unref(runtime.loop);
        return EXIT_FAILURE;
    }
    if (runtime.ready_fd >= 0) {
        const guint8 started = 2;
        (void)write(runtime.ready_fd, &started, sizeof started);
    }
    (void)guardian_send_status(runtime.guardian_status_fd, GUARDIAN_STATUS_STARTED);

    gnoblin_config_set_runtime_wakeup_callback(runtime_schedule_deferred_callbacks, &runtime);
    g_unix_fd_add(runtime.channel_fd, G_IO_IN | G_IO_HUP | G_IO_ERR | G_IO_NVAL,
                  runtime_channel_ready, &runtime);
    runtime.startup_timeout_id = g_timeout_add(STARTUP_TIMEOUT_MS, startup_timeout, &runtime);
    g_main_loop_run(runtime.loop);
    if (runtime.startup_timeout_id)
        g_source_remove(runtime.startup_timeout_id);
    if (runtime.write_watch_id)
        g_source_remove(runtime.write_watch_id);
    if (runtime.policy_event_idle_id)
        g_source_remove(runtime.policy_event_idle_id);
    if (runtime.deferred_packets_idle_id)
        g_source_remove(runtime.deferred_packets_idle_id);
    if (runtime.deferred_callback_idle_id)
        g_source_remove(runtime.deferred_callback_idle_id);
    if (runtime.reload_progress_id)
        g_source_remove(runtime.reload_progress_id);
    close(runtime.channel_fd);
    close(runtime.guardian_status_fd);
    gnoblin_runtime_writer_free(runtime.writer);
    gnoblin_runtime_reader_free(runtime.reader);
    gnoblin_config_set_focus_policy_changed_callback(NULL, NULL);
    gnoblin_config_set_permission_policy_changed_callback(NULL, NULL);
    gnoblin_config_set_runtime_wakeup_callback(NULL, NULL);
    g_queue_clear_full(&runtime.policy_events, runtime_policy_event_free);
    g_clear_pointer(&runtime.initial_document, g_variant_unref);
    if (runtime.host_control_fd >= 0)
        close(runtime.host_control_fd);
    g_queue_clear_full(&runtime.deferred_packets, runtime_deferred_packet_free);
    if (runtime.pending_reload) {
        gnoblin_config_finish_deferred_load(FALSE);
        runtime_reload_free(runtime.pending_reload);
    }
    g_hash_table_unref(runtime.state_revisions);
    g_hash_table_unref(runtime.pending_logout_ids);
    g_hash_table_unref(runtime.pending_operation_ids);
    g_main_loop_unref(runtime.loop);
    return runtime.resume_rejected      ? EXIT_RESUME_REJECTED
           : runtime.signal_exit_status ? runtime.signal_exit_status
           : runtime.failed             ? EXIT_FAILURE
                                        : runtime.exit_status;
}

static volatile sig_atomic_t host_signal_number;

static void host_signal_handler(int signal_number) {
    host_signal_number = signal_number;
}

static gboolean move_fd_above_runtime_targets(int fd, int* moved_fd, GError** error) {
    int moved = fcntl(fd, F_DUPFD_CLOEXEC, WORKER_GUARDIAN_STATUS_FD + 1);
    if (moved < 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "could not move worker descriptor: %s", g_strerror(errno));
        return FALSE;
    }
    *moved_fd = moved;
    return TRUE;
}

static GPid spawn_runtime_worker(const char* executable, const char* config_path, gboolean devkit,
                                 const char* wayland_display, gboolean resume,
                                 gboolean send_autostart, guint64 settings_revision,
                                 guint64 runtime_generation, guint64 operation_id_watermark,
                                 int channel_fd, int host_control_fd, int guardian_status_fd,
                                 int* ready_fd, GError** error) {
    int status_pipe[2] = {-1, -1};
    int channel_parent_alias = -1, channel_child_alias = -1;
    int status_parent_alias = -1, status_child_alias = -1;
    int host_parent_alias = -1, host_child_alias = -1, guardian_status_alias = -1;
    if (pipe2(status_pipe, O_CLOEXEC | O_NONBLOCK) != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "could not create worker status pipe: %s", g_strerror(errno));
        return 0;
    }
    if (!move_fd_above_runtime_targets(channel_fd, &channel_parent_alias, error) ||
        !move_fd_above_runtime_targets(channel_fd, &channel_child_alias, error) ||
        !move_fd_above_runtime_targets(status_pipe[0], &status_parent_alias, error) ||
        !move_fd_above_runtime_targets(status_pipe[1], &status_child_alias, error) ||
        !move_fd_above_runtime_targets(guardian_status_fd, &guardian_status_alias, error) ||
        (host_control_fd >= 0 &&
         (!move_fd_above_runtime_targets(host_control_fd, &host_parent_alias, error) ||
          !move_fd_above_runtime_targets(host_control_fd, &host_child_alias, error))))
        goto fail;

    posix_spawn_file_actions_t actions;
    int result = posix_spawn_file_actions_init(&actions);
    if (result != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(result),
                    "could not prepare runtime worker spawn: %s", g_strerror(result));
        goto fail;
    }
    int channel_temporary_fd = -1, status_temporary_fd = -1;
    int host_temporary_fd = -1;
    gboolean actions_ready =
        gnoblin_runtime_spawn_add_channel_actions(&actions, channel_parent_alias,
                                                  channel_child_alias, RUNTIME_FD,
                                                  &channel_temporary_fd, error) &&
        gnoblin_runtime_spawn_add_channel_actions(&actions, status_parent_alias, status_child_alias,
                                                  WORKER_READY_FD, &status_temporary_fd, error) &&
        (host_control_fd < 0 ||
         gnoblin_runtime_spawn_add_channel_actions(&actions, host_parent_alias, host_child_alias,
                                                   WORKER_HOST_FD, &host_temporary_fd, error));
    if (actions_ready) {
        int action_result = posix_spawn_file_actions_adddup2(&actions, guardian_status_alias,
                                                             WORKER_GUARDIAN_STATUS_FD);
        if (action_result == 0)
            action_result = posix_spawn_file_actions_addclose(&actions, guardian_status_alias);
        if (action_result == 0)
            action_result = posix_spawn_file_actions_addclose(&actions, GUARDIAN_STATUS_FD);
        if (action_result == 0)
            action_result = posix_spawn_file_actions_addclose(&actions, GUARDIAN_AUTOSTART_FD);
        if (action_result != 0) {
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(action_result),
                        "could not prepare guardian status descriptor: %s",
                        g_strerror(action_result));
            actions_ready = FALSE;
        }
    }
    if (!actions_ready) {
        posix_spawn_file_actions_destroy(&actions);
        if (channel_temporary_fd >= 0)
            close(channel_temporary_fd);
        if (status_temporary_fd >= 0)
            close(status_temporary_fd);
        if (host_temporary_fd >= 0)
            close(host_temporary_fd);
        goto fail;
    }

    g_autofree char* ready_arg = g_strdup_printf("%d", WORKER_READY_FD);
    g_autofree char* revision_arg = g_strdup_printf("%" G_GUINT64_FORMAT, settings_revision);
    g_autofree char* generation_arg = g_strdup_printf("%" G_GUINT64_FORMAT, runtime_generation);
    g_autofree char* operation_id_watermark_arg =
        g_strdup_printf("%" G_GUINT64_FORMAT, operation_id_watermark);
    g_autofree char* host_control_arg = g_strdup_printf("%d", WORKER_HOST_FD);
    g_autoptr(GPtrArray) argv = g_ptr_array_new_with_free_func(g_free);
    g_ptr_array_add(argv, g_strdup(executable));
    g_ptr_array_add(argv, g_strdup("--internal-runtime-worker"));
    g_ptr_array_add(argv, g_strdup("--config"));
    g_ptr_array_add(argv, g_strdup(config_path));
    g_ptr_array_add(argv, g_strdup("--worker-ready-fd"));
    g_ptr_array_add(argv, g_strdup(ready_arg));
    if (host_control_fd >= 0) {
        g_ptr_array_add(argv, g_strdup("--worker-host-fd"));
        g_ptr_array_add(argv, g_strdup(host_control_arg));
    }
    if (resume) {
        g_ptr_array_add(argv, g_strdup("--resume-runtime-worker"));
        g_ptr_array_add(argv, g_strdup("--seed-settings-revision"));
        g_ptr_array_add(argv, g_strdup(revision_arg));
        g_ptr_array_add(argv, g_strdup("--seed-runtime-generation"));
        g_ptr_array_add(argv, g_strdup(generation_arg));
        g_ptr_array_add(argv, g_strdup("--seed-operation-id-watermark"));
        g_ptr_array_add(argv, g_strdup(operation_id_watermark_arg));
    }
    if (send_autostart)
        g_ptr_array_add(argv, g_strdup("--send-initial-autostart"));
    if (devkit) {
        g_ptr_array_add(argv, g_strdup("--devkit"));
        g_ptr_array_add(argv, g_strdup("--wayland-display"));
        g_ptr_array_add(argv, g_strdup(wayland_display));
    }
    g_ptr_array_add(argv, NULL);
    pid_t pid = 0;
    result = posix_spawn(&pid, executable, &actions, NULL, (char**)argv->pdata, environ);
    posix_spawn_file_actions_destroy(&actions);
    if (channel_temporary_fd >= 0)
        close(channel_temporary_fd);
    if (status_temporary_fd >= 0)
        close(status_temporary_fd);
    if (host_temporary_fd >= 0)
        close(host_temporary_fd);
    if (guardian_status_alias >= 0)
        close(guardian_status_alias);
    close(channel_parent_alias);
    close(channel_child_alias);
    close(status_parent_alias);
    close(status_child_alias);
    if (host_parent_alias >= 0)
        close(host_parent_alias);
    if (host_child_alias >= 0)
        close(host_child_alias);
    close(status_pipe[1]);
    if (result != 0) {
        close(status_pipe[0]);
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(result),
                    "could not start runtime worker: %s", g_strerror(result));
        return 0;
    }
    *ready_fd = status_pipe[0];
    return (GPid)pid;

fail:
    if (channel_parent_alias >= 0)
        close(channel_parent_alias);
    if (channel_child_alias >= 0)
        close(channel_child_alias);
    if (status_parent_alias >= 0)
        close(status_parent_alias);
    if (status_child_alias >= 0)
        close(status_child_alias);
    if (host_parent_alias >= 0)
        close(host_parent_alias);
    if (host_child_alias >= 0)
        close(host_child_alias);
    if (guardian_status_alias >= 0)
        close(guardian_status_alias);
    close(status_pipe[0]);
    close(status_pipe[1]);
    return 0;
}

static gboolean drain_worker_ready_fd(int fd, gboolean* start_packet_sent) {
    guint8 bytes[16];
    gboolean ready = FALSE;
    for (;;) {
        ssize_t count = read(fd, bytes, sizeof bytes);
        if (count > 0) {
            for (ssize_t i = 0; i < count; i++) {
                if (bytes[i] == 1)
                    ready = TRUE;
                if (bytes[i] == 2)
                    *start_packet_sent = TRUE;
            }
            continue;
        }
        if (count < 0 && errno == EINTR)
            continue;
        break;
    }
    return ready;
}

static gboolean send_runtime_control_packet(int channel_fd, GnoblinRuntimePacketType type,
                                            const char* notification, GError** error) {
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    g_autoptr(GVariant) empty =
        g_variant_ref_sink(g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
    if (!gnoblin_runtime_writer_queue(writer, type, 0, empty, error))
        return FALSE;
    gint64 deadline = g_get_monotonic_time() + SUSPEND_TIMEOUT_MS * 1000;
    for (;;) {
        g_autoptr(GError) flush_error = NULL;
        if (gnoblin_runtime_writer_flush(writer, channel_fd, &flush_error))
            return TRUE;
        if (!g_error_matches(flush_error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK)) {
            g_propagate_error(error, g_steal_pointer(&flush_error));
            return FALSE;
        }
        gint64 remaining = deadline - g_get_monotonic_time();
        if (remaining <= 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_TIMED_OUT, notification);
            return FALSE;
        }
        struct pollfd pfd = {.fd = channel_fd, .events = POLLOUT};
        int timeout = (int)MIN((remaining + 999) / 1000, 100);
        if (poll(&pfd, 1, timeout) < 0 && errno != EINTR) {
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                        "could not wait to notify Mutter: %s", g_strerror(errno));
            return FALSE;
        }
    }
}

static gboolean send_worker_disconnected(int channel_fd, GError** error) {
    return send_runtime_control_packet(channel_fd, GNOBLIN_RUNTIME_PACKET_WORKER_DISCONNECTED,
                                       "Mutter did not accept worker disconnect notification",
                                       error);
}

static void notify_runtime_recovery_failed(int channel_fd) {
    g_autoptr(GError) error = NULL;
    if (!send_runtime_control_packet(channel_fd, GNOBLIN_RUNTIME_PACKET_RECOVERY_FAILED,
                                     "Mutter did not accept runtime recovery failure notification",
                                     &error))
        g_printerr("gnoblin: could not report terminal runtime recovery failure: %s\n",
                   error ? error->message : "notification failed");
}

static gboolean wait_for_worker_suspended(int channel_fd, Runtime* host, guint64* settings_revision,
                                          guint64* runtime_generation,
                                          guint64* operation_id_watermark, GError** error) {
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    gint64 deadline = g_get_monotonic_time() + SUSPEND_TIMEOUT_MS * 1000;
    while (!host_signal_number) {
        if (host_reap_compositor(host)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                                "Mutter exited while suspending the Lua worker");
            return FALSE;
        }
        gint64 remaining = deadline - g_get_monotonic_time();
        if (remaining <= 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_TIMED_OUT,
                                "Mutter did not acknowledge worker suspension");
            return FALSE;
        }
        struct pollfd pfd = {.fd = channel_fd, .events = POLLIN | POLLHUP | POLLERR};
        int timeout = (int)MIN((remaining + 999) / 1000, 100);
        int polled = poll(&pfd, 1, timeout);
        if (polled < 0) {
            if (errno == EINTR)
                continue;
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                        "could not wait for Mutter suspension: %s", g_strerror(errno));
            return FALSE;
        }
        if (!polled)
            continue;
        if (pfd.revents & (POLLHUP | POLLERR | POLLNVAL)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                                "Mutter closed the runtime channel during suspension");
            return FALSE;
        }
        for (;;) {
            GnoblinRuntimePacket packet = {0};
            gboolean available = FALSE;
            if (!gnoblin_runtime_reader_receive(reader, channel_fd, &packet, &available, error))
                return FALSE;
            if (!available)
                break;
            if (packet.type == GNOBLIN_RUNTIME_PACKET_WORKER_SUSPENDED) {
                gboolean valid =
                    packet.request_id == 0 &&
                    g_variant_lookup(packet.payload, "settings_revision", "t", settings_revision) &&
                    g_variant_lookup(packet.payload, "runtime_generation", "t",
                                     runtime_generation) &&
                    g_variant_lookup(packet.payload, "operation_id_watermark", "t",
                                     operation_id_watermark) &&
                    *settings_revision > 0 && *runtime_generation > 0 &&
                    *operation_id_watermark <= G_MAXINT64;
                gnoblin_runtime_packet_clear(&packet);
                if (!valid) {
                    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                        "Mutter sent an invalid worker suspension acknowledgement");
                    return FALSE;
                }
                return TRUE;
            }
            gnoblin_runtime_packet_clear(&packet);
        }
    }
    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CANCELLED,
                        "session host received a termination signal");
    return FALSE;
}

static gboolean sleep_before_worker_restart(guint restart_number, Runtime* host) {
    guint delay_ms = MIN(250u << MIN(restart_number - 1, 4u), 4000u);
    guint elapsed = 0;
    while (elapsed < delay_ms && !host_signal_number) {
        if (host_reap_compositor(host))
            return FALSE;
        guint step = MIN(100u, delay_ms - elapsed);
        g_usleep(step * 1000);
        elapsed += step;
    }
    return !host_signal_number;
}

typedef struct {
    GPid pid;
    char* name;
} HostAutostartChild;

typedef struct {
    char* name;
    char** argv;
} HostAutostartEntry;

static void host_autostart_entry_free(gpointer data) {
    HostAutostartEntry* entry = data;
    g_free(entry->name);
    g_strfreev(entry->argv);
    g_free(entry);
}

static void host_autostart_child_free(gpointer data) {
    HostAutostartChild* child = data;
    g_spawn_close_pid(child->pid);
    g_free(child->name);
    g_free(child);
}

static gboolean launch_initial_autostart(GVariant* entries, GVariant* environment,
                                         GPtrArray* children, GError** error) {
    g_autoptr(GHashTable) names = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autoptr(GPtrArray) parsed_entries = g_ptr_array_new_with_free_func(host_autostart_entry_free);
    g_auto(GStrv) child_environment = gnoblin_autostart_build_environment(environment, error);
    if (!child_environment)
        return FALSE;
    if (!entries || !g_variant_is_of_type(entries, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Lua runtime did not provide an autostart list");
        return FALSE;
    }
    gsize index = 0;
    for (; index < g_variant_n_children(entries); index++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(entries, index);
        g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
        g_autoptr(GVariant) command = NULL;
        g_autoptr(GVariant) when_value = NULL;
        const char* name = NULL;
        if (!g_variant_is_of_type(entry, G_VARIANT_TYPE_VARDICT) ||
            !g_variant_lookup(entry, "name", "&s", &name) || !name || !*name ||
            g_utf8_strlen(name, -1) > 80 || g_hash_table_contains(names, name))
            goto invalid_entry;
        GVariantIter fields;
        const char* key;
        GVariant* value;
        g_variant_iter_init(&fields, entry);
        while (g_variant_iter_next(&fields, "{&sv}", &key, &value)) {
            gboolean supported =
                g_str_equal(key, "name") || g_str_equal(key, "command") || g_str_equal(key, "when");
            g_variant_unref(value);
            if (!supported)
                goto invalid_entry;
        }
        when_value = g_variant_lookup_value(entry, "when", NULL);
        if (when_value && (!g_variant_is_of_type(when_value, G_VARIANT_TYPE_STRING) ||
                           !g_str_equal(g_variant_get_string(when_value, NULL), "on_login")))
            goto invalid_entry;
        command = g_variant_lookup_value(entry, "command", G_VARIANT_TYPE("av"));
        if (!command || g_variant_n_children(command) == 0)
            goto invalid_entry;
        gsize argc = g_variant_n_children(command);
        g_auto(GStrv) argv = g_new0(char*, argc + 1);
        for (gsize arg = 0; arg < argc; arg++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(command, arg);
            g_autoptr(GVariant) argument = g_variant_get_variant(wrapped);
            if (!g_variant_is_of_type(argument, G_VARIANT_TYPE_STRING))
                goto invalid_entry;
            argv[arg] = g_variant_dup_string(argument, NULL);
        }
        if (!argv[0] || !*argv[0])
            goto invalid_entry;

        g_hash_table_add(names, g_strdup(name));
        HostAutostartEntry* parsed = g_new0(HostAutostartEntry, 1);
        parsed->name = g_strdup(name);
        parsed->argv = g_steal_pointer(&argv);
        g_ptr_array_add(parsed_entries, parsed);
    }

    for (guint index = 0; index < parsed_entries->len; index++) {
        HostAutostartEntry* entry = g_ptr_array_index(parsed_entries, index);
        GPid child_pid = 0;
        g_autoptr(GError) spawn_error = NULL;
        if (!g_spawn_async(NULL, entry->argv, child_environment,
                           G_SPAWN_SEARCH_PATH | G_SPAWN_DO_NOT_REAP_CHILD, NULL, NULL, &child_pid,
                           &spawn_error)) {
            g_warning("gnoblin-autostart: could not start %s: %s", entry->name,
                      spawn_error->message);
            continue;
        }
        HostAutostartChild* child = g_new0(HostAutostartChild, 1);
        child->pid = child_pid;
        child->name = g_strdup(entry->name);
        g_ptr_array_add(children, child);
        g_message("gnoblin-autostart: started %s", entry->name);
    }
    return TRUE;

invalid_entry:
    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                "Lua runtime sent invalid autostart entry %zu", index + 1);
    return FALSE;
}

static gboolean guardian_send_status(int fd, guint8 status) {
    ssize_t written;
    do {
        written = send(fd, &status, sizeof status, MSG_NOSIGNAL);
    } while (written < 0 && errno == EINTR);
    return written == sizeof status;
}

static gboolean forward_autostart_to_guardian(int fd, GVariant* entries, GVariant* environment,
                                              GError** error) {
    GVariantBuilder payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload, "{sv}", "entries", entries);
    g_variant_builder_add(&payload, "{sv}", "environment", environment);
    g_autoptr(GVariant) packet_payload = g_variant_ref_sink(g_variant_builder_end(&payload));
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    if (!gnoblin_runtime_writer_queue(writer, GNOBLIN_RUNTIME_PACKET_HOST_AUTOSTART, 0,
                                      packet_payload, error))
        return FALSE;
    return gnoblin_runtime_writer_flush(writer, fd, error);
}

static void reap_autostart_children(GPtrArray* children) {
    for (guint index = 0; index < children->len;) {
        HostAutostartChild* child = g_ptr_array_index(children, index);
        int status = 0;
        pid_t result = waitpid(child->pid, &status, WNOHANG);
        if (result == 0) {
            index++;
            continue;
        }
        if (result < 0 && errno == EINTR)
            continue;
        if (result < 0 || !WIFEXITED(status) || WEXITSTATUS(status) != 0)
            g_warning("gnoblin-autostart: %s exited unsuccessfully", child->name);
        g_ptr_array_remove_index_fast(children, index);
    }
}

static GPid spawn_session_supervisor(const char* executable, const char* config_path,
                                     gboolean devkit, const char* wayland_display, int runtime_fd,
                                     int status_parent_fd, int status_child_fd,
                                     int autostart_parent_fd, int autostart_child_fd,
                                     gboolean resume, guint64 settings_revision,
                                     guint64 runtime_generation, guint64 operation_id_watermark,
                                     gboolean autostart_complete, GError** error) {
    int runtime_alias = -1, status_parent_alias = -1, status_child_alias = -1;
    int autostart_parent_alias = -1, autostart_child_alias = -1;
    if (!move_fd_above_runtime_targets(runtime_fd, &runtime_alias, error) ||
        !move_fd_above_runtime_targets(status_parent_fd, &status_parent_alias, error) ||
        !move_fd_above_runtime_targets(status_child_fd, &status_child_alias, error) ||
        !move_fd_above_runtime_targets(autostart_parent_fd, &autostart_parent_alias, error) ||
        !move_fd_above_runtime_targets(autostart_child_fd, &autostart_child_alias, error))
        goto fail;

    posix_spawn_file_actions_t actions;
    int result = posix_spawn_file_actions_init(&actions);
    if (result != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(result),
                    "could not prepare session supervisor spawn: %s", g_strerror(result));
        goto fail;
    }
    int runtime_result = posix_spawn_file_actions_adddup2(&actions, runtime_alias, RUNTIME_FD);
    if (runtime_result == 0)
        runtime_result = posix_spawn_file_actions_addclose(&actions, runtime_alias);
    gboolean actions_ready =
        runtime_result == 0 &&
        gnoblin_runtime_spawn_add_channel_actions(&actions, status_parent_alias, status_child_alias,
                                                  GUARDIAN_STATUS_FD, &(int){-1}, error) &&
        gnoblin_runtime_spawn_add_channel_actions(&actions, autostart_parent_alias,
                                                  autostart_child_alias, GUARDIAN_AUTOSTART_FD,
                                                  &(int){-1}, error);
    if (runtime_result != 0 && (!error || !*error))
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(runtime_result),
                    "could not prepare session compositor descriptor: %s",
                    g_strerror(runtime_result));
    if (!actions_ready) {
        posix_spawn_file_actions_destroy(&actions);
        goto fail;
    }

    posix_spawnattr_t attributes;
    result = posix_spawnattr_init(&attributes);
    if (result != 0) {
        posix_spawn_file_actions_destroy(&actions);
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(result),
                    "could not prepare session supervisor process group: %s", g_strerror(result));
        goto fail;
    }
    result = posix_spawnattr_setpgroup(&attributes, 0);
    if (result == 0)
        result = posix_spawnattr_setflags(&attributes, POSIX_SPAWN_SETPGROUP);
    if (result != 0) {
        posix_spawnattr_destroy(&attributes);
        posix_spawn_file_actions_destroy(&actions);
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(result),
                    "could not prepare session supervisor process group: %s", g_strerror(result));
        goto fail;
    }

    g_autofree char* revision_arg = g_strdup_printf("%" G_GUINT64_FORMAT, settings_revision);
    g_autofree char* generation_arg = g_strdup_printf("%" G_GUINT64_FORMAT, runtime_generation);
    g_autofree char* watermark_arg = g_strdup_printf("%" G_GUINT64_FORMAT, operation_id_watermark);
    g_autoptr(GPtrArray) argv = g_ptr_array_new_with_free_func(g_free);
    g_ptr_array_add(argv, g_strdup(executable));
    g_ptr_array_add(argv, g_strdup("--internal-session-supervisor"));
    g_ptr_array_add(argv, g_strdup("--config"));
    g_ptr_array_add(argv, g_strdup(config_path));
    if (devkit) {
        g_ptr_array_add(argv, g_strdup("--devkit"));
        g_ptr_array_add(argv, g_strdup("--wayland-display"));
        g_ptr_array_add(argv, g_strdup(wayland_display));
    }
    if (resume) {
        g_ptr_array_add(argv, g_strdup("--resume-session-supervisor"));
        g_ptr_array_add(argv, g_strdup("--resume-settings-revision"));
        g_ptr_array_add(argv, g_strdup(revision_arg));
        g_ptr_array_add(argv, g_strdup("--resume-runtime-generation"));
        g_ptr_array_add(argv, g_strdup(generation_arg));
        g_ptr_array_add(argv, g_strdup("--resume-operation-id-watermark"));
        g_ptr_array_add(argv, g_strdup(watermark_arg));
    }
    if (autostart_complete)
        g_ptr_array_add(argv, g_strdup("--autostart-complete"));
    g_ptr_array_add(argv, NULL);

    pid_t pid = 0;
    result = posix_spawn(&pid, executable, &actions, &attributes, (char**)argv->pdata, environ);
    posix_spawnattr_destroy(&attributes);
    posix_spawn_file_actions_destroy(&actions);
    if (result != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(result),
                    "could not start Gnoblin session supervisor: %s", g_strerror(result));
        goto fail;
    }
    close(runtime_alias);
    close(status_parent_alias);
    close(status_child_alias);
    close(autostart_parent_alias);
    close(autostart_child_alias);
    return (GPid)pid;

fail:
    if (runtime_alias >= 0)
        close(runtime_alias);
    if (status_parent_alias >= 0)
        close(status_parent_alias);
    if (status_child_alias >= 0)
        close(status_child_alias);
    if (autostart_parent_alias >= 0)
        close(autostart_parent_alias);
    if (autostart_child_alias >= 0)
        close(autostart_child_alias);
    return 0;
}

static int session_supervisor_main(int argc, char** argv) {
    g_autofree char* config_path = NULL;
    const char* wayland_display = NULL;
    gboolean devkit = FALSE;
    gboolean resume_supervisor = FALSE;
    gboolean autostart_complete = FALSE;
    guint64 supervisor_seed_revision = 0, supervisor_seed_generation = 0;
    guint64 supervisor_seed_operation_id_watermark = 0;
    for (int i = 1; i < argc; i++) {
        if (g_str_equal(argv[i], "--config") && i + 1 < argc)
            config_path = g_canonicalize_filename(argv[++i], NULL);
        else if (g_str_equal(argv[i], "--devkit"))
            devkit = TRUE;
        else if (g_str_equal(argv[i], "--wayland-display") && i + 1 < argc)
            wayland_display = argv[++i];
        else if (g_str_equal(argv[i], "--internal-session-supervisor"))
            ;
        else if (g_str_equal(argv[i], "--resume-session-supervisor"))
            resume_supervisor = TRUE;
        else if (g_str_equal(argv[i], "--autostart-complete"))
            autostart_complete = TRUE;
        else if (g_str_equal(argv[i], "--resume-settings-revision") && i + 1 < argc)
            supervisor_seed_revision = g_ascii_strtoull(argv[++i], NULL, 10);
        else if (g_str_equal(argv[i], "--resume-runtime-generation") && i + 1 < argc)
            supervisor_seed_generation = g_ascii_strtoull(argv[++i], NULL, 10);
        else if (g_str_equal(argv[i], "--resume-operation-id-watermark") && i + 1 < argc)
            supervisor_seed_operation_id_watermark = g_ascii_strtoull(argv[++i], NULL, 10);
        else {
            usage(argv[0]);
            return EXIT_FAILURE;
        }
    }
    if (devkit != (wayland_display != NULL) ||
        (wayland_display &&
         (!g_regex_match_simple("^[A-Za-z0-9_.-]{1,128}$", wayland_display, 0, 0) ||
          g_str_equal(wayland_display, ".") || g_str_equal(wayland_display, "..")))) {
        usage(argv[0]);
        g_printerr("gnoblin: devkit mode needs a valid Wayland display name\n");
        return EXIT_FAILURE;
    }
    if (!config_path)
        config_path = gnoblin_config_path();
    g_setenv("GNOBLIN_CONFIG", config_path, TRUE);
    if (fcntl(RUNTIME_FD, F_GETFD) < 0 || fcntl(GUARDIAN_STATUS_FD, F_GETFD) < 0 ||
        fcntl(GUARDIAN_AUTOSTART_FD, F_GETFD) < 0 ||
        (resume_supervisor && (!supervisor_seed_revision || !supervisor_seed_generation))) {
        g_printerr("gnoblin: invalid inherited session supervisor channels\n");
        return EXIT_FAILURE;
    }

    struct sigaction action = {.sa_handler = host_signal_handler};
    sigemptyset(&action.sa_mask);
    sigaction(SIGTERM, &action, NULL);
    sigaction(SIGINT, &action, NULL);
    g_autoptr(GError) error = NULL;
    Runtime host = {.channel_fd = RUNTIME_FD, .exit_status = EXIT_FAILURE};
    g_autofree char* executable = g_file_read_link("/proc/self/exe", NULL);
    if (!executable) {
        g_printerr("gnoblin: could not locate installed executable\n");
        return EXIT_FAILURE;
    }

    GPid worker_pid = 0;
    int ready_fd = -1;
    gboolean worker_ready = FALSE;
    gboolean worker_start_packet_sent = FALSE;
    gboolean first_worker = TRUE;
    gboolean host_autostart_received = FALSE;
    gboolean host_autostart_started = FALSE;
    gboolean stop_session = FALSE;
    gboolean leave_session_alive = FALSE;
    gboolean explicit_logout = FALSE;
    guint restart_count = 0;
    gint64 worker_ready_since_us = 0;
    guint64 resume_revision = 0, resume_generation = 0;
    guint64 resume_operation_id_watermark = 0;
    int host_control_sockets[2] = {-1, -1};
    if (!autostart_complete &&
        socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, host_control_sockets) != 0) {
        g_printerr("gnoblin: could not create private runtime supervisor channel: %s\n",
                   g_strerror(errno));
        return EXIT_FAILURE;
    }
    int host_control_fd = host_control_sockets[0];
    g_autoptr(GnoblinRuntimeReader) host_control_reader = gnoblin_runtime_reader_new();
    g_autoptr(GVariant) host_autostart_entries = NULL;
    g_autoptr(GVariant) host_autostart_environment = NULL;
    host_autostart_started = autostart_complete;
    worker_pid = spawn_runtime_worker(
        executable, config_path, devkit, wayland_display, resume_supervisor, !autostart_complete,
        supervisor_seed_revision, supervisor_seed_generation,
        supervisor_seed_operation_id_watermark, host.channel_fd, host_control_sockets[1],
        GUARDIAN_STATUS_FD, &ready_fd, &error);
    if (host_control_sockets[1] >= 0) {
        close(host_control_sockets[1]);
        host_control_sockets[1] = -1;
    }
    if (!worker_pid) {
        g_printerr("gnoblin: could not start Lua runtime worker: %s\n",
                   error ? error->message : "spawn failed");
        close(host_control_fd);
        return EXIT_FAILURE;
    }

    while (!host_signal_number) {
        if (host_control_fd >= 0) {
            struct pollfd control_poll = {.fd = host_control_fd, .events = POLLIN | POLLHUP};
            if (poll(&control_poll, 1, 0) > 0 && (control_poll.revents & (POLLIN | POLLHUP))) {
                if (!gnoblin_autostart_receive_packet(
                        host_control_fd, host_control_reader, &host_autostart_received,
                        &host_autostart_entries, &host_autostart_environment, &error)) {
                    g_printerr("gnoblin: Lua runtime supervisor channel failed: %s\n",
                               error ? error->message : "invalid packet");
                    stop_session = TRUE;
                    break;
                }
            }
        }
        if (worker_pid) {
            struct pollfd ready_poll = {.fd = ready_fd, .events = POLLIN | POLLHUP};
            if (poll(&ready_poll, 1, 0) > 0 && (ready_poll.revents & (POLLIN | POLLHUP))) {
                gboolean became_ready = drain_worker_ready_fd(ready_fd, &worker_start_packet_sent);
                worker_ready |= became_ready;
                if (became_ready && !worker_ready_since_us)
                    worker_ready_since_us = g_get_monotonic_time();
                if (became_ready && !host_autostart_started) {
                    if (host_control_fd >= 0 &&
                        !gnoblin_autostart_receive_packet(
                            host_control_fd, host_control_reader, &host_autostart_received,
                            &host_autostart_entries, &host_autostart_environment, &error)) {
                        g_printerr("gnoblin: Lua runtime supervisor channel failed: %s\n",
                                   error ? error->message : "invalid packet");
                        stop_session = TRUE;
                        break;
                    }
                    if (!host_autostart_received ||
                        !forward_autostart_to_guardian(GUARDIAN_AUTOSTART_FD,
                                                       host_autostart_entries,
                                                       host_autostart_environment, &error)) {
                        g_printerr("gnoblin: could not start login autostart: %s\n",
                                   error ? error->message
                                         : "Lua runtime did not send the initial list");
                        stop_session = TRUE;
                        break;
                    }
                    host_autostart_started = TRUE;
                    close(host_control_fd);
                    host_control_fd = -1;
                }
            }
            if (worker_ready_since_us &&
                g_get_monotonic_time() - worker_ready_since_us >= WORKER_STABLE_RESET_MS * 1000)
                restart_count = 0;
            int worker_status = 0;
            pid_t worker_result = waitpid(worker_pid, &worker_status, WNOHANG);
            if (worker_result == worker_pid) {
                gboolean became_ready = drain_worker_ready_fd(ready_fd, &worker_start_packet_sent);
                worker_ready |= became_ready;
                if (became_ready && !worker_ready_since_us)
                    worker_ready_since_us = g_get_monotonic_time();
                close(ready_fd);
                ready_fd = -1;
                worker_pid = 0;
                g_spawn_close_pid((GPid)worker_result);
                gboolean clean_exit =
                    WIFEXITED(worker_status) && WEXITSTATUS(worker_status) == EXIT_SUCCESS;
                if (clean_exit) {
                    stop_session = TRUE;
                    explicit_logout = TRUE;
                    host.exit_status = EXIT_SUCCESS;
                    break;
                }
                if (WIFEXITED(worker_status) &&
                    WEXITSTATUS(worker_status) == EXIT_RESUME_REJECTED) {
                    g_printerr("gnoblin: Mutter rejected runtime resume; keeping the session host "
                               "and compositor alive\n");
                    leave_session_alive = TRUE;
                    break;
                }
                if (first_worker && !worker_ready) {
                    g_printerr("gnoblin: Lua runtime failed before Mutter handshake\n");
                    stop_session = TRUE;
                    host.exit_status = EXIT_FAILURE;
                    break;
                }
                first_worker = FALSE;
                restart_count++;
                /* If a replacement failed before sending RESUME, Mutter is
                 * still in the SUSPENDED state already acknowledged above. */
                if (worker_start_packet_sent) {
                    g_autoptr(GError) suspend_error = NULL;
                    if (!send_worker_disconnected(host.channel_fd, &suspend_error) ||
                        !wait_for_worker_suspended(
                            host.channel_fd, &host, &resume_revision, &resume_generation,
                            &resume_operation_id_watermark, &suspend_error)) {
                        if (!host.compositor_pid) {
                            stop_session = TRUE;
                            break;
                        }
                        g_printerr("gnoblin: could not complete worker suspension; keeping the "
                                   "session host and compositor alive: %s\n",
                                   suspend_error ? suspend_error->message
                                                 : "Mutter did not suspend");
                        notify_runtime_recovery_failed(host.channel_fd);
                        leave_session_alive = TRUE;
                        break;
                    }
                }
                if (restart_count > WORKER_MAX_RESTARTS) {
                    g_printerr("gnoblin: Lua runtime recovery failed repeatedly; leaving Mutter "
                               "and session targets running\n");
                    notify_runtime_recovery_failed(host.channel_fd);
                    leave_session_alive = TRUE;
                    break;
                }
                g_printerr("gnoblin: restarting Lua runtime worker (attempt %u of %u)\n",
                           restart_count, WORKER_MAX_RESTARTS);
                if (!sleep_before_worker_restart(restart_count, &host)) {
                    stop_session = TRUE;
                    break;
                }
                worker_ready = FALSE;
                worker_start_packet_sent = FALSE;
                worker_ready_since_us = 0;
                worker_pid = spawn_runtime_worker(
                    executable, config_path, devkit, wayland_display, TRUE, !host_autostart_started,
                    resume_revision, resume_generation, resume_operation_id_watermark,
                    host.channel_fd, -1, GUARDIAN_STATUS_FD, &ready_fd, &error);
                if (!worker_pid) {
                    g_printerr("gnoblin: could not restart Lua runtime worker: %s\n",
                               error ? error->message : "spawn failed");
                    notify_runtime_recovery_failed(host.channel_fd);
                    leave_session_alive = TRUE;
                    break;
                }
            }
        }
        g_usleep(50000);
    }

    if (host_signal_number) {
        host.exit_status = 128 + host_signal_number;
        stop_session = TRUE;
    }
    if (leave_session_alive && !host_signal_number) {
        /* Exhausted recovery keeps the session host alive so logind ownership,
         * Mutter, and session targets continue until logout or compositor exit. */
        while (!host_signal_number) {
            if (host_reap_compositor(&host)) {
                stop_session = TRUE;
                break;
            }
            g_usleep(100000);
        }
    }
    if (host_signal_number) {
        host.exit_status = 128 + host_signal_number;
        stop_session = TRUE;
    }
    if (explicit_logout)
        (void)guardian_send_status(GUARDIAN_STATUS_FD, GUARDIAN_STATUS_LOGOUT);
    if (worker_pid) {
        kill(worker_pid, SIGTERM);
        while (waitpid(worker_pid, NULL, 0) < 0 && errno == EINTR) {
        }
        g_spawn_close_pid(worker_pid);
    }
    if (ready_fd >= 0)
        close(ready_fd);
    if (host_control_fd >= 0)
        close(host_control_fd);
    if (host_control_sockets[1] >= 0)
        close(host_control_sockets[1]);
    close(host.channel_fd);
    return host.exit_status;
}

static gboolean guardian_drain_status(int fd, gboolean* native_channel_started,
                                      gboolean* supervisor_ready, gboolean* session_ready,
                                      gint64* ready_since_us, gboolean* explicit_logout,
                                      GError** error) {
    for (;;) {
        guint8 status = 0;
        ssize_t count = recv(fd, &status, sizeof status, MSG_DONTWAIT);
        if (count == 0)
            return TRUE;
        if (count < 0) {
            if (errno == EINTR)
                continue;
            if (errno == EAGAIN || errno == EWOULDBLOCK)
                return TRUE;
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                        "could not read session supervisor status: %s", g_strerror(errno));
            return FALSE;
        }
        if (count != sizeof status) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "session supervisor sent a truncated status message");
            return FALSE;
        }
        switch (status) {
        case GUARDIAN_STATUS_STARTED:
            *native_channel_started = TRUE;
            break;
        case GUARDIAN_STATUS_READY:
            if (!*supervisor_ready)
                *ready_since_us = g_get_monotonic_time();
            *supervisor_ready = TRUE;
            *session_ready = TRUE;
            break;
        case GUARDIAN_STATUS_LOGOUT:
            *explicit_logout = TRUE;
            break;
        default:
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                        "session supervisor sent unknown status %u", status);
            return FALSE;
        }
    }
}

static gboolean guardian_receive_autostart(int fd, GnoblinRuntimeReader* reader, gboolean* received,
                                           gboolean* autostart_complete, GVariant** entries,
                                           GVariant** environment, GPtrArray* children,
                                           GError** error) {
    gboolean was_received = *received;
    if (!gnoblin_autostart_receive_packet(fd, reader, received, entries, environment, error)) {
        if (!*received && error && *error &&
            g_error_matches(*error, G_IO_ERROR, G_IO_ERROR_CLOSED)) {
            g_clear_error(error);
            return TRUE;
        }
        return FALSE;
    }
    if (!was_received && *received && !*autostart_complete) {
        /* Record the one-shot transition before launching anything. If the
         * supervisor exits immediately afterwards, a replacement must not
         * run the same login commands a second time. */
        *autostart_complete = TRUE;
        if (!launch_initial_autostart(*entries, *environment, children, error))
            return FALSE;
    }
    return TRUE;
}

static void guardian_stop_supervisor(GPid* supervisor_pid, gboolean graceful) {
    if (!supervisor_pid || !*supervisor_pid)
        return;
    GPid pid = *supervisor_pid;
    *supervisor_pid = 0;
    kill(-pid, graceful ? SIGTERM : SIGKILL);
    int status = 0;
    gboolean reaped = FALSE;
    for (guint i = 0; i < (graceful ? 200u : 10u); i++) {
        pid_t result = waitpid(pid, &status, WNOHANG);
        if (result == pid || (result < 0 && errno == ECHILD)) {
            reaped = TRUE;
            break;
        }
        if (result < 0 && errno != EINTR)
            break;
        g_usleep(10000);
    }
    if (!reaped) {
        kill(-pid, SIGKILL);
        while (waitpid(pid, &status, 0) < 0 && errno == EINTR) {
        }
    }
    g_spawn_close_pid(pid);
}

static int session_guardian_main(int argc, char** argv) {
    g_autofree char* compositor_path = g_canonicalize_filename(GNOBLIN_DEFAULT_COMPOSITOR, NULL);
    g_autofree char* plugin = g_strdup("libgnoblin");
    g_autofree char* config_path = NULL;
    const char* wayland_display = NULL;
    g_autofree char* xwayland_program = g_find_program_in_path("Xwayland");
    gboolean xwayland = xwayland_program != NULL;
    gboolean devkit = FALSE;
    for (int i = 1; i < argc; i++) {
        if (g_str_equal(argv[i], "--config") && i + 1 < argc)
            config_path = g_canonicalize_filename(argv[++i], NULL);
        else if (g_str_equal(argv[i], "--devkit"))
            devkit = TRUE;
        else if (g_str_equal(argv[i], "--wayland-display") && i + 1 < argc)
            wayland_display = argv[++i];
        else if (g_str_equal(argv[i], "--no-xwayland"))
            xwayland = FALSE;
        else if (g_str_equal(argv[i], "--xwayland"))
            xwayland = TRUE;
        else {
            usage(argv[0]);
            return EXIT_FAILURE;
        }
    }
    if (devkit != (wayland_display != NULL) ||
        (wayland_display &&
         (!g_regex_match_simple("^[A-Za-z0-9_.-]{1,128}$", wayland_display, 0, 0) ||
          g_str_equal(wayland_display, ".") || g_str_equal(wayland_display, "..")))) {
        usage(argv[0]);
        g_printerr("gnoblin: devkit mode needs a valid Wayland display name\n");
        return EXIT_FAILURE;
    }
    if (!devkit && !prepare_session_environment())
        return EXIT_FAILURE;
    if (!config_path)
        config_path = gnoblin_config_path();
    g_setenv("GNOBLIN_CONFIG", config_path, TRUE);
    g_autoptr(GError) config_error = NULL;
    g_autoptr(GVariant) initial_document =
        gnoblin_config_load_document(config_path, NULL, NULL, &config_error);
    if (!initial_document) {
        g_printerr("gnoblin: could not load %s: %s\n", config_path,
                   config_error ? config_error->message : "invalid configuration");
        return EXIT_FAILURE;
    }
    g_autoptr(GError) portal_error = NULL;
    if (!gnoblin_config_sync_portal_selection(initial_document, NULL, &portal_error))
        g_warning("gnoblin: could not update portal selection from Lua config: %s",
                  portal_error ? portal_error->message : "unknown error");
    if (!g_file_test(compositor_path, G_FILE_TEST_IS_EXECUTABLE)) {
        g_printerr("gnoblin: installed compositor is unavailable: %s\n", compositor_path);
        return EXIT_FAILURE;
    }
    if (!devkit && !activate_session())
        return EXIT_FAILURE;

    /* The guardian owns the lifecycle name even while supervisors restart. */
    g_autoptr(GDBusConnection) lifecycle_bus = claim_session_lifecycle_bus();
    struct sigaction action = {.sa_handler = host_signal_handler};
    sigemptyset(&action.sa_mask);
    sigaction(SIGTERM, &action, NULL);
    sigaction(SIGINT, &action, NULL);

    int sockets[2] = {-1, -1};
    if (socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets) != 0) {
        g_printerr("gnoblin: could not create private compositor channel: %s\n", g_strerror(errno));
        return EXIT_FAILURE;
    }
    g_autoptr(GError) error = NULL;
    Runtime guardian = {.channel_fd = sockets[0], .exit_status = EXIT_FAILURE};
    guardian.compositor_pid = spawn_compositor(compositor_path, plugin, sockets[0], sockets[1],
                                               xwayland, devkit, wayland_display, &error);
    close(sockets[1]);
    if (!guardian.compositor_pid) {
        close(sockets[0]);
        g_printerr("gnoblin: %s\n", error ? error->message : "compositor spawn failed");
        return EXIT_FAILURE;
    }
    g_autofree char* executable = g_file_read_link("/proc/self/exe", NULL);
    if (!executable) {
        g_printerr("gnoblin: could not locate installed executable\n");
        terminate_and_reap(&guardian);
        close(guardian.channel_fd);
        return EXIT_FAILURE;
    }

    GPid supervisor_pid = 0;
    int status_sockets[2] = {-1, -1};
    int autostart_sockets[2] = {-1, -1};
    int status_fd = -1, autostart_fd = -1;
    g_autoptr(GnoblinRuntimeReader) autostart_reader = gnoblin_runtime_reader_new();
    g_autoptr(GVariant) autostart_entries = NULL;
    g_autoptr(GVariant) autostart_environment = NULL;
    g_autoptr(GPtrArray) autostart_children =
        g_ptr_array_new_with_free_func(host_autostart_child_free);
    gboolean autostart_received = FALSE;
    gboolean autostart_complete = FALSE;
    gboolean native_channel_started = FALSE;
    gboolean supervisor_ready = FALSE;
    gboolean session_ready = FALSE;
    gboolean explicit_logout = FALSE;
    gboolean stop_session = FALSE;
    gboolean retain_session = FALSE;
    gboolean resume_supervisor = FALSE;
    guint restart_count = 0;
    gint64 supervisor_ready_since_us = 0;
    guint64 resume_revision = 0, resume_generation = 0, resume_operation_id_watermark = 0;

    while (!host_signal_number && !stop_session) {
        if (!supervisor_pid) {
            if (socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, status_sockets) != 0 ||
                socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, autostart_sockets) != 0) {
                g_printerr("gnoblin: could not create private supervisor channels: %s\n",
                           g_strerror(errno));
                guardian.exit_status = EXIT_FAILURE;
                stop_session = TRUE;
                break;
            }
            status_fd = status_sockets[0];
            autostart_fd = autostart_sockets[0];
            supervisor_pid = spawn_session_supervisor(
                executable, config_path, devkit, wayland_display, guardian.channel_fd,
                status_sockets[0], status_sockets[1], autostart_sockets[0], autostart_sockets[1],
                resume_supervisor, resume_revision, resume_generation,
                resume_operation_id_watermark, autostart_complete, &error);
            close(status_sockets[1]);
            status_sockets[1] = -1;
            close(autostart_sockets[1]);
            autostart_sockets[1] = -1;
            if (!supervisor_pid) {
                g_printerr("gnoblin: could not start session supervisor: %s\n",
                           error ? error->message : "spawn failed");
                guardian.exit_status = EXIT_FAILURE;
                stop_session = TRUE;
                break;
            }
            error = NULL;
        }

        struct pollfd polls[2] = {{.fd = status_fd, .events = POLLIN | POLLHUP},
                                  {.fd = autostart_fd, .events = POLLIN | POLLHUP}};
        (void)poll(polls, G_N_ELEMENTS(polls), 50);
        if (!guardian_drain_status(status_fd, &native_channel_started, &supervisor_ready,
                                   &session_ready, &supervisor_ready_since_us, &explicit_logout,
                                   &error)) {
            g_printerr("gnoblin: session supervisor status failed: %s\n",
                       error ? error->message : "invalid message");
            stop_session = TRUE;
            break;
        }
        if (!guardian_receive_autostart(autostart_fd, autostart_reader, &autostart_received,
                                        &autostart_complete, &autostart_entries,
                                        &autostart_environment, autostart_children, &error)) {
            g_printerr("gnoblin: login autostart data failed: %s\n",
                       error ? error->message : "invalid message");
            stop_session = TRUE;
            break;
        }
        if (host_reap_compositor(&guardian)) {
            stop_session = TRUE;
            break;
        }

        int supervisor_status = 0;
        pid_t supervisor_result = waitpid(supervisor_pid, &supervisor_status, WNOHANG);
        if (supervisor_result < 0 && errno != EINTR) {
            g_printerr("gnoblin: could not wait for session supervisor: %s\n", g_strerror(errno));
            guardian.exit_status = EXIT_FAILURE;
            stop_session = TRUE;
            break;
        }
        if (supervisor_result != supervisor_pid) {
            reap_autostart_children(autostart_children);
            if (supervisor_ready && supervisor_ready_since_us &&
                g_get_monotonic_time() - supervisor_ready_since_us >= WORKER_STABLE_RESET_MS * 1000)
                restart_count = 0;
            continue;
        }

        g_spawn_close_pid(supervisor_pid);
        supervisor_pid = 0;
        kill(-supervisor_result, SIGKILL);
        if (!guardian_drain_status(status_fd, &native_channel_started, &supervisor_ready,
                                   &session_ready, &supervisor_ready_since_us, &explicit_logout,
                                   &error) ||
            !guardian_receive_autostart(autostart_fd, autostart_reader, &autostart_received,
                                        &autostart_complete, &autostart_entries,
                                        &autostart_environment, autostart_children, &error)) {
            g_printerr("gnoblin: could not collect final supervisor state: %s\n",
                       error ? error->message : "invalid message");
            stop_session = TRUE;
        }
        close(status_fd);
        close(autostart_fd);
        status_fd = autostart_fd = -1;
        status_sockets[0] = autostart_sockets[0] = -1;
        supervisor_ready = FALSE;
        supervisor_ready_since_us = 0;

        if (stop_session)
            break;
        if (host_signal_number || explicit_logout) {
            stop_session = TRUE;
            guardian.exit_status = explicit_logout ? EXIT_SUCCESS : 128 + host_signal_number;
            break;
        }
        if (WIFEXITED(supervisor_status) &&
            WEXITSTATUS(supervisor_status) == EXIT_RESUME_REJECTED) {
            g_printerr("gnoblin: Mutter rejected supervisor recovery; keeping the session alive\n");
            notify_runtime_recovery_failed(guardian.channel_fd);
            retain_session = TRUE;
            break;
        }
        if (host_reap_compositor(&guardian)) {
            stop_session = TRUE;
            break;
        }

        if (native_channel_started) {
            g_autoptr(GError) suspend_error = NULL;
            if (!send_worker_disconnected(guardian.channel_fd, &suspend_error) ||
                !wait_for_worker_suspended(guardian.channel_fd, &guardian, &resume_revision,
                                           &resume_generation, &resume_operation_id_watermark,
                                           &suspend_error)) {
                if (host_reap_compositor(&guardian)) {
                    stop_session = TRUE;
                    break;
                }
                g_printerr("gnoblin: could not suspend Mutter for supervisor recovery; keeping the "
                           "session alive: %s\n",
                           suspend_error ? suspend_error->message : "no acknowledgement");
                notify_runtime_recovery_failed(guardian.channel_fd);
                retain_session = TRUE;
                break;
            }
            native_channel_started = FALSE;
            resume_supervisor = TRUE;
        }
        restart_count++;
        if (restart_count > WORKER_MAX_RESTARTS) {
            g_printerr("gnoblin: session supervisor recovery failed repeatedly; keeping Mutter "
                       "and the session alive\n");
            notify_runtime_recovery_failed(guardian.channel_fd);
            retain_session = TRUE;
            break;
        }
        g_printerr("gnoblin: restarting session supervisor (attempt %u of %u)\n", restart_count,
                   WORKER_MAX_RESTARTS);
        if (!sleep_before_worker_restart(restart_count, &guardian)) {
            stop_session = TRUE;
            break;
        }
        supervisor_ready_since_us = 0;
        g_clear_pointer(&autostart_reader, gnoblin_runtime_reader_free);
        autostart_reader = gnoblin_runtime_reader_new();
    }

    if (retain_session && !host_signal_number) {
        while (!host_signal_number) {
            if (host_reap_compositor(&guardian)) {
                stop_session = TRUE;
                break;
            }
            reap_autostart_children(autostart_children);
            g_usleep(100000);
        }
    }
    if (host_signal_number) {
        guardian.exit_status = 128 + host_signal_number;
        stop_session = TRUE;
    }
    if (supervisor_pid)
        guardian_stop_supervisor(&supervisor_pid, TRUE);
    if (status_fd >= 0)
        close(status_fd);
    if (autostart_fd >= 0)
        close(autostart_fd);
    if (status_sockets[1] >= 0)
        close(status_sockets[1]);
    if (autostart_sockets[1] >= 0)
        close(autostart_sockets[1]);
    if (stop_session && session_ready)
        notify_portal_session_ending(lifecycle_bus);
    reap_autostart_children(autostart_children);
    if (stop_session && guardian.compositor_pid)
        terminate_and_reap(&guardian);
    close(guardian.channel_fd);
    return guardian.exit_status;
}

int main(int argc, char** argv) {
    for (int i = 1; i < argc; i++) {
        if (g_str_equal(argv[i], "--internal-runtime-worker"))
            return runtime_worker_main(argc, argv);
        if (g_str_equal(argv[i], "--internal-session-supervisor"))
            return session_supervisor_main(argc, argv);
    }
    if (argc >= 2 && g_str_equal(argv[1], "--version"))
        return runtime_worker_main(argc, argv);
    return session_guardian_main(argc, argv);
}
