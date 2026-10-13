#include "gnoblin-runtime-internal.h"

static void host_autostart_entry_free(gpointer data) {
    HostAutostartEntry* entry = data;
    g_free(entry->name);
    g_strfreev(entry->argv);
    g_free(entry);
}

void host_autostart_child_free(gpointer data) {
    HostAutostartChild* child = data;
    g_spawn_close_pid(child->pid);
    g_free(child->name);
    g_free(child);
}

static GPtrArray* parse_autostart_entries(GVariant* entries, GError** error) {
    g_autoptr(GHashTable) names = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    GPtrArray* parsed_entries = g_ptr_array_new_with_free_func(host_autostart_entry_free);
    if (!entries || !g_variant_is_of_type(entries, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Lua runtime did not provide an autostart list");
        goto invalid_entries;
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

    return parsed_entries;

invalid_entry:
    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                "Lua runtime sent invalid autostart entry %zu", index + 1);
invalid_entries:
    g_ptr_array_unref(parsed_entries);
    return NULL;
}

static gboolean autostart_entry_equal(const HostAutostartEntry* left,
                                      const HostAutostartEntry* right) {
    return left && right && g_str_equal(left->name, right->name) &&
           g_strv_equal((const char* const*)left->argv, (const char* const*)right->argv);
}

static HostAutostartEntry* autostart_entry_find(GPtrArray* entries, const char* name) {
    for (guint index = 0; entries && index < entries->len; index++) {
        HostAutostartEntry* entry = g_ptr_array_index(entries, index);
        if (g_str_equal(entry->name, name))
            return entry;
    }
    return NULL;
}

static HostAutostartChild* autostart_child_find(GPtrArray* children, const char* name,
                                                guint* index_out) {
    for (guint index = 0; children && index < children->len; index++) {
        HostAutostartChild* child = g_ptr_array_index(children, index);
        if (g_str_equal(child->name, name)) {
            if (index_out)
                *index_out = index;
            return child;
        }
    }
    return NULL;
}

static void autostart_child_setup(gpointer user_data) {
    (void)user_data;

    /* Every configured command gets its own process group. This keeps the
     * lifecycle owned by Gnoblin even when a launcher leaves a layer client
     * running after its direct child has exited. */
    (void)setpgid(0, 0);
}

static gboolean autostart_process_group_exists(GPid process_group) {
    if (process_group <= 0)
        return FALSE;
    if (kill(-process_group, 0) == 0 || errno == EPERM)
        return TRUE;
    return errno != ESRCH;
}

static void stop_autostart_child(HostAutostartChild* child) {
    if (!child || child->pid <= 0)
        return;
    gboolean has_process_group = autostart_process_group_exists(child->process_group);
    pid_t target = has_process_group ? -child->process_group : child->pid;
    if (kill(target, SIGTERM) != 0 && errno != ESRCH)
        g_warning("gnoblin-autostart: could not stop %s: %s", child->name, g_strerror(errno));
    int status = 0;
    for (guint attempt = 0; attempt < 10; attempt++) {
        pid_t result = 0;
        if (!child->leader_reaped) {
            result = waitpid(child->pid, &status, WNOHANG);
            if (result == child->pid || (result < 0 && errno == ECHILD))
                child->leader_reaped = TRUE;
        }
        if ((!has_process_group && child->leader_reaped) ||
            (has_process_group && !autostart_process_group_exists(child->process_group)))
            return;
        if (result < 0 && errno != EINTR) {
            g_warning("gnoblin-autostart: could not reap %s: %s", child->name, g_strerror(errno));
            return;
        }
        g_usleep(20 * 1000);
    }
    if (kill(target, SIGKILL) != 0 && errno != ESRCH)
        g_warning("gnoblin-autostart: could not force-stop %s: %s", child->name, g_strerror(errno));
    if (!child->leader_reaped) {
        while (waitpid(child->pid, &status, 0) < 0 && errno == EINTR)
            ;
        child->leader_reaped = TRUE;
    }
}

gboolean reconcile_autostart(GVariant* entries, GVariant* environment, GPtrArray* children,
                             GPtrArray** active_entries, GError** error) {
    g_autoptr(GPtrArray) next = parse_autostart_entries(entries, error);
    if (!next)
        return FALSE;
    g_auto(GStrv) child_environment = gnoblin_autostart_build_environment(environment, error);
    if (!child_environment)
        return FALSE;

    /* Stop only process groups Gnoblin launched. XDG autostart and all other
     * session processes remain outside this registry. */
    for (guint index = 0; index < children->len;) {
        HostAutostartChild* child = g_ptr_array_index(children, index);
        HostAutostartEntry* old = autostart_entry_find(*active_entries, child->name);
        HostAutostartEntry* replacement = autostart_entry_find(next, child->name);
        if (!replacement || !autostart_entry_equal(old, replacement)) {
            stop_autostart_child(child);
            g_ptr_array_remove_index(children, index);
            continue;
        }
        index++;
    }
    for (guint index = 0; index < next->len; index++) {
        HostAutostartEntry* entry = g_ptr_array_index(next, index);
        if (autostart_child_find(children, entry->name, NULL))
            continue;
        GPid child_pid = 0;
        g_autoptr(GError) spawn_error = NULL;
        if (!g_spawn_async(NULL, entry->argv, child_environment,
                           G_SPAWN_SEARCH_PATH | G_SPAWN_DO_NOT_REAP_CHILD, autostart_child_setup,
                           NULL, &child_pid, &spawn_error)) {
            g_warning("gnoblin-autostart: could not start %s: %s", entry->name,
                      spawn_error->message);
            g_autofree char* diagnostic =
                g_strdup_printf("Could not start %s: %s", entry->name, spawn_error->message);
            append_config_notice(diagnostic);
            continue;
        }
        HostAutostartChild* child = g_new0(HostAutostartChild, 1);
        child->pid = child_pid;
        /* The child sets this before exec. Record the intended group without
         * querying getpgid(): the parent can otherwise win that race. */
        child->process_group = child_pid;
        child->name = g_strdup(entry->name);
        g_ptr_array_add(children, child);
        g_message("gnoblin-autostart: started %s", entry->name);
    }
    g_clear_pointer(active_entries, g_ptr_array_unref);
    *active_entries = g_steal_pointer(&next);
    return TRUE;
}

static gint compare_string_pointers(gconstpointer left, gconstpointer right) {
    return g_strcmp0(*(char* const*)left, *(char* const*)right);
}

void start_xdg_autostart(GVariant* environment) {
    const char* wayland_display = NULL;
    const char* display = NULL;
    const char* xauthority = NULL;
    GVariantIter iter;
    const char* key;
    const char* value;
    g_variant_iter_init(&iter, environment);
    while (g_variant_iter_next(&iter, "{&s&s}", &key, &value)) {
        if (g_str_equal(key, "WAYLAND_DISPLAY"))
            wayland_display = value;
        else if (g_str_equal(key, "DISPLAY"))
            display = value;
        else if (g_str_equal(key, "XAUTHORITY"))
            xauthority = value;
    }
    if (!wayland_display) {
        g_warning(
            "gnoblin-autostart: Mutter did not provide WAYLAND_DISPLAY; skipping XDG autostart");
        return;
    }

    g_autoptr(GError) bus_error = NULL;
    g_autoptr(GDBusConnection) bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &bus_error);
    if (bus) {
        GVariantBuilder activation_environment;
        g_variant_builder_init(&activation_environment, G_VARIANT_TYPE("a{ss}"));
        g_variant_builder_add(&activation_environment, "{ss}", "WAYLAND_DISPLAY", wayland_display);
        if (display)
            g_variant_builder_add(&activation_environment, "{ss}", "DISPLAY", display);
        if (xauthority)
            g_variant_builder_add(&activation_environment, "{ss}", "XAUTHORITY", xauthority);
        g_autoptr(GVariant) reply = g_dbus_connection_call_sync(
            bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
            "UpdateActivationEnvironment",
            g_variant_new("(@a{ss})", g_variant_builder_end(&activation_environment)),
            G_VARIANT_TYPE("()"), G_DBUS_CALL_FLAGS_NONE, 2000, NULL, &bus_error);
        if (!reply)
            g_warning("gnoblin-autostart: could not update D-Bus activation environment: %s",
                      bus_error->message);
    } else {
        g_warning("gnoblin-autostart: session D-Bus is unavailable: %s", bus_error->message);
    }

    /* XDG autostart belongs to the session manager. Read the standard config
     * directories here after Mutter publishes its display, without requiring
     * a particular process manager or unit system. */
    g_autoptr(GHashTable) files = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
    const gchar* const* system_config_dirs = g_get_system_config_dirs();
    gsize system_count = g_strv_length((gchar**)system_config_dirs);
    for (gssize index = (gssize)system_count - 1; index >= 0; index--) {
        g_autofree char* directory = g_build_filename(system_config_dirs[index], "autostart", NULL);
        g_autoptr(GDir) dir = g_dir_open(directory, 0, NULL);
        if (!dir)
            continue;
        const char* name;
        while ((name = g_dir_read_name(dir))) {
            if (g_str_has_suffix(name, ".desktop"))
                g_hash_table_replace(files, g_strdup(name),
                                     g_build_filename(directory, name, NULL));
        }
    }
    g_autofree char* user_autostart = g_build_filename(g_get_user_config_dir(), "autostart", NULL);
    g_autoptr(GDir) user_dir = g_dir_open(user_autostart, 0, NULL);
    if (user_dir) {
        const char* name;
        while ((name = g_dir_read_name(user_dir))) {
            if (g_str_has_suffix(name, ".desktop"))
                g_hash_table_replace(files, g_strdup(name),
                                     g_build_filename(user_autostart, name, NULL));
        }
    }

    g_autoptr(GPtrArray) names = g_ptr_array_new_with_free_func(g_free);
    GHashTableIter file_iter;
    gpointer file_name;
    g_hash_table_iter_init(&file_iter, files);
    while (g_hash_table_iter_next(&file_iter, &file_name, NULL))
        g_ptr_array_add(names, g_strdup(file_name));
    g_ptr_array_sort(names, compare_string_pointers);

    for (guint index = 0; index < names->len; index++) {
        const char* name = g_ptr_array_index(names, index);
        const char* path = g_hash_table_lookup(files, name);
        g_autoptr(GDesktopAppInfo) app = g_desktop_app_info_new_from_filename(path);
        if (!app || g_desktop_app_info_get_is_hidden(app) ||
            (g_desktop_app_info_has_key(app, "X-GNOME-Autostart-enabled") &&
             !g_desktop_app_info_get_boolean(app, "X-GNOME-Autostart-enabled")) ||
            !g_desktop_app_info_get_show_in(app, "Gnoblin:GNOME"))
            continue;
        g_autoptr(GAppLaunchContext) context = g_app_launch_context_new();
        g_app_launch_context_setenv(context, "WAYLAND_DISPLAY", wayland_display);
        if (display)
            g_app_launch_context_setenv(context, "DISPLAY", display);
        if (xauthority)
            g_app_launch_context_setenv(context, "XAUTHORITY", xauthority);
        g_autoptr(GError) error = NULL;
        if (!g_app_info_launch(G_APP_INFO(app), NULL, context, &error))
            g_warning("gnoblin-autostart: could not launch %s: %s", name, error->message);
        else
            g_message("gnoblin-autostart: launched %s", name);
    }
}

gboolean guardian_send_status(int fd, guint8 status) {
    ssize_t written;
    do {
        written = send(fd, &status, sizeof status, MSG_NOSIGNAL);
    } while (written < 0 && errno == EINTR);
    return written == sizeof status;
}

gboolean forward_autostart_to_guardian(int fd, GVariant* entries, GVariant* environment,
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

void reap_autostart_children(GPtrArray* children) {
    for (guint index = 0; index < children->len;) {
        HostAutostartChild* child = g_ptr_array_index(children, index);
        if (child->leader_reaped) {
            if (autostart_process_group_exists(child->process_group)) {
                index++;
                continue;
            }
            g_ptr_array_remove_index_fast(children, index);
            continue;
        }
        int status = 0;
        pid_t result = waitpid(child->pid, &status, WNOHANG);
        int wait_error = errno;
        if (result == 0) {
            index++;
            continue;
        }
        if (result < 0 && errno == EINTR)
            continue;
        if (result < 0 || !WIFEXITED(status) || WEXITSTATUS(status) != 0) {
            g_warning("gnoblin-autostart: %s exited unsuccessfully", child->name);
            g_autofree char* diagnostic = NULL;
            if (result < 0)
                diagnostic = g_strdup_printf("Could not monitor %s: %s", child->name,
                                             g_strerror(wait_error));
            else if (WIFSIGNALED(status))
                diagnostic =
                    g_strdup_printf("%s crashed (signal %d)", child->name, WTERMSIG(status));
            else
                diagnostic = g_strdup_printf("%s exited with status %d", child->name,
                                             WIFEXITED(status) ? WEXITSTATUS(status) : -1);
            append_config_notice(diagnostic);
        }
        child->leader_reaped = TRUE;
        if (!autostart_process_group_exists(child->process_group))
            g_ptr_array_remove_index_fast(children, index);
        else
            index++;
    }
}

void stop_autostart_children(GPtrArray* children) {
    for (guint index = 0; children && index < children->len; index++)
        stop_autostart_child(g_ptr_array_index(children, index));
}
