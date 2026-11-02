#include "gnoblin-runtime-internal.h"

GVariant* read_recovery_snapshot(int fd, GError** error) {
    struct stat statbuf;
    if (fd < 0 || fstat(fd, &statbuf) != 0 || statbuf.st_size <= 0 ||
        statbuf.st_size > GNOBLIN_RUNTIME_PROTOCOL_MAX_PAYLOAD) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "invalid inherited recovery snapshot");
        return NULL;
    }
    gsize size = (gsize)statbuf.st_size;
    guint8* data = g_malloc(size);
    gsize offset = 0;
    while (offset < size) {
        ssize_t count = pread(fd, data + offset, size - offset, offset);
        if (count < 0 && errno == EINTR)
            continue;
        if (count <= 0) {
            g_free(data);
            g_set_error(error, G_IO_ERROR,
                        count < 0 ? g_io_error_from_errno(errno) : G_IO_ERROR_INVALID_DATA,
                        "could not read inherited recovery snapshot");
            return NULL;
        }
        offset += count;
    }
    g_autoptr(GBytes) bytes = g_bytes_new_take(data, size);
    GVariant* snapshot =
        g_variant_ref_sink(g_variant_new_from_bytes(G_VARIANT_TYPE_VARDICT, bytes, FALSE));
    if (!g_variant_is_normal_form(snapshot)) {
        g_variant_unref(snapshot);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "recovery snapshot is not in normal form");
        return NULL;
    }
    return snapshot;
}

int runtime_worker_main(int argc, char** argv) {
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
    gboolean config_fallback = FALSE;
    gboolean config_fallback_last_good = FALSE;
    g_autofree char* config_fallback_error = NULL;
    /* Only the supervisor sets this on a replacement child. Do not retry a
     * crashing user configuration in the process that must recover it. */
    g_autofree char* startup_recovery = g_strdup(g_getenv("GNOBLIN_INTERNAL_STARTUP_RECOVERY"));
    g_unsetenv("GNOBLIN_INTERNAL_STARTUP_RECOVERY");
    g_autoptr(GVariant) recovery_snapshot = NULL;
    g_autoptr(GVariant) recovery_document = NULL;
    g_autoptr(GVariant) recovery_events = NULL;
    if (fcntl(WORKER_RECOVERY_SNAPSHOT_FD, F_GETFD) >= 0) {
        recovery_snapshot = read_recovery_snapshot(WORKER_RECOVERY_SNAPSHOT_FD, &error);
        close(WORKER_RECOVERY_SNAPSHOT_FD);
        if (!recovery_snapshot || !resume_worker ||
            !(recovery_document =
                  g_variant_lookup_value(recovery_snapshot, "document", G_VARIANT_TYPE_VARDICT)) ||
            !(recovery_events = g_variant_lookup_value(recovery_snapshot, "runtime_events",
                                                       G_VARIANT_TYPE("as")))) {
            g_printerr("gnoblin: invalid recovery snapshot\n");
            return EXIT_FAILURE;
        }
    }
    g_autoptr(GVariant) document = NULL;
    g_autoptr(GPtrArray) ignored_settings = NULL;
    if (recovery_snapshot) {
        config_fallback = TRUE;
        g_autofree char* previous_notice = config_recovery_notice();
        const char* restart_notice =
            startup_recovery ? startup_recovery
                             : "The Lua runtime restarted; built-in defaults are being applied";
        config_fallback_error = previous_notice && *previous_notice
                                    ? g_strdup_printf("%s\n%s", previous_notice, restart_notice)
                                    : g_strdup(restart_notice);
        document = gnoblin_config_load_runtime_recovery_snapshot(config_path, recovery_document,
                                                                 &paths, &directories, &error);
    } else if (startup_recovery) {
        config_fallback = TRUE;
        config_fallback_error = g_strdup(startup_recovery);
        document = gnoblin_config_load_runtime_defaults(&paths, &directories, &error);
    } else if (!resume_worker) {
        /* Native bootstrap never receives unaccepted user settings. */
        document = gnoblin_config_load_runtime_defaults(&paths, &directories, &error);
    } else {
        /* Keep every valid setting. Only a broken key or list entry is ignored and reported below.
         */
        document = gnoblin_config_load_runtime_salvaged(config_path, &paths, &directories,
                                                        &ignored_settings, &error);
    }
    if (!document && !startup_recovery) {
        g_autofree char* failure = g_strdup(error ? error->message : "invalid configuration");
        config_fallback_error = g_strdup(failure);
        gnoblin_config_finish_load(FALSE);
        g_clear_error(&error);
        config_fallback = TRUE;
        g_autoptr(GVariant) fallback =
            startup_fallback_document(config_path, failure, &config_fallback_last_good);
        document = config_fallback_last_good
                       ? gnoblin_config_load_runtime_fallback(config_path, fallback, &paths,
                                                              &directories, &error)
                       : gnoblin_config_load_runtime_defaults(&paths, &directories, &error);
        if (!document && config_fallback_last_good) {
            g_autofree char* combined =
                g_strdup_printf("%s\nSaved configuration also failed: %s", config_fallback_error,
                                error ? error->message : "runtime initialization failed");
            g_free(config_fallback_error);
            config_fallback_error = g_steal_pointer(&combined);
            gnoblin_config_finish_load(FALSE);
            g_clear_error(&error);
            g_clear_pointer(&paths, g_ptr_array_unref);
            g_clear_pointer(&directories, g_ptr_array_unref);
            config_fallback_last_good = FALSE;
            document = gnoblin_config_load_runtime_defaults(&paths, &directories, &error);
        }
    }
    if (!document) {
        g_printerr("gnoblin: could not start with fallback configuration: %s\n",
                   error ? error->message : "runtime initialization failed");
        gnoblin_config_finish_load(FALSE);
        return EXIT_FAILURE;
    }
    /* Publish before native bootstrap, rather than only after HELLO. */
    if (config_fallback)
        set_config_fallback_marker(TRUE, config_fallback_last_good, config_fallback_error);
    if (ignored_settings && ignored_settings->len > 0) {
        /* The rest of the configuration is active. Say plainly what was left out, in the session
         * log and in the notice the panel reads. */
        g_autoptr(GString) notice =
            g_string_new("Some settings were ignored because they are invalid:");
        for (guint i = 0; i < ignored_settings->len; i++) {
            const char* line = g_ptr_array_index(ignored_settings, i);
            g_printerr("gnoblin: %s\n", line);
            g_string_append_printf(notice, "\n%s", line);
        }
        append_config_notice(notice->str);
    }
    gnoblin_config_finish_load(TRUE);
    guint64 revision = gnoblin_config_settings_revision();

    GStatBuf user_config_stat;
    /* System drop-ins are applied after the safe bootstrap, like the user config. */
    gboolean user_config_present = g_lstat(config_path, &user_config_stat) == 0 ||
                                   errno != ENOENT || gnoblin_config_has_system_dropins();
    Runtime runtime = {
        .channel_fd = RUNTIME_FD,
        .session_log_fd = -1,
        .compositor_stderr_fd = -1,
        .ready_fd = ready_fd,
        .host_control_fd = host_control_fd,
        .guardian_status_fd = WORKER_GUARDIAN_STATUS_FD,
        .initial_document = send_autostart && !recovery_snapshot ? g_variant_ref(document) : NULL,
        .resume_worker = resume_worker,
        .send_initial_autostart = send_autostart && !recovery_snapshot,
        .config_fallback = config_fallback || !resume_worker || recovery_snapshot != NULL,
        .startup_config_pending = recovery_snapshot != NULL ||
                                  (!resume_worker && !startup_recovery && user_config_present),
        .recovery_snapshot = recovery_snapshot != NULL,
        .recovery_runtime_events = recovery_events ? g_variant_ref(recovery_events) : NULL,
        .config_fallback_last_good = config_fallback_last_good,
        .config_fallback_error = config_fallback_error,
        .exit_status = EXIT_FAILURE,
        .writer = gnoblin_runtime_writer_new(),
        .reader = gnoblin_runtime_reader_new(),
        .state_revisions = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free),
        .pending_logout_ids = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL),
        .pending_operation_ids = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL)};
    if (recovery_snapshot)
        runtime.startup_failure =
            g_strdup(config_fallback_error
                         ? config_fallback_error
                         : "The Lua runtime restarted; built-in defaults are being applied");
    g_queue_init(&runtime.policy_events);
    g_queue_init(&runtime.deferred_packets);
    gnoblin_config_set_focus_policy_changed_callback(focus_policy_committed, &runtime);
    runtime.loop = g_main_loop_new(NULL, FALSE);
    gnoblin_priority_notice_start();
    RuntimeSignal sigterm = {.runtime = &runtime, .signal_number = SIGTERM};
    RuntimeSignal sigint = {.runtime = &runtime, .signal_number = SIGINT};
    g_unix_signal_add(SIGTERM, runtime_signal_received, &sigterm);
    g_unix_signal_add(SIGINT, runtime_signal_received, &sigint);
    if (fcntl(RUNTIME_FD, F_GETFD) < 0) {
        gnoblin_runtime_writer_free(runtime.writer);
        gnoblin_runtime_reader_free(runtime.reader);
        gnoblin_config_set_focus_policy_changed_callback(NULL, NULL);
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
        g_hash_table_unref(runtime.state_revisions);
        g_hash_table_unref(runtime.pending_logout_ids);
        g_hash_table_unref(runtime.pending_operation_ids);
        g_main_loop_unref(runtime.loop);
        return EXIT_FAILURE;
    }
    g_autoptr(GVariant) resume_payload =
        resume_worker ? resume_packet_payload(normalized_document, revision,
                                              gnoblin_config_runtime_generation(),
                                              seed_operation_id_watermark, recovery_events)
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
    if (runtime.startup_config_idle_id)
        g_source_remove(runtime.startup_config_idle_id);
    g_free(runtime.startup_failure);
    close(runtime.channel_fd);
    close(runtime.guardian_status_fd);
    gnoblin_runtime_writer_free(runtime.writer);
    gnoblin_runtime_reader_free(runtime.reader);
    gnoblin_config_set_focus_policy_changed_callback(NULL, NULL);
    gnoblin_config_set_runtime_wakeup_callback(NULL, NULL);
    g_queue_clear_full(&runtime.policy_events, runtime_policy_event_free);
    g_clear_pointer(&runtime.initial_document, g_variant_unref);
    g_clear_pointer(&runtime.session_environment, g_variant_unref);
    g_clear_pointer(&runtime.recovery_runtime_events, g_variant_unref);
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

volatile sig_atomic_t host_signal_number;

void host_signal_handler(int signal_number) {
    host_signal_number = signal_number;
}

gboolean move_fd_above_runtime_targets(int fd, int* moved_fd, GError** error) {
    int moved = fcntl(fd, F_DUPFD_CLOEXEC, WORKER_RECOVERY_SNAPSHOT_FD + 1);
    if (moved < 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "could not move worker descriptor: %s", g_strerror(errno));
        return FALSE;
    }
    *moved_fd = moved;
    return TRUE;
}

int create_recovery_snapshot_fd(GVariant* snapshot, GError** error) {
    if (!snapshot || !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_is_normal_form(snapshot)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "worker recovery snapshot is not a normal a{sv} variant");
        return -1;
    }
    g_autoptr(GBytes) bytes = g_variant_get_data_as_bytes(snapshot);
    gsize length = 0;
    const guint8* data = g_bytes_get_data(bytes, &length);
    if (!data || length == 0 || length > GNOBLIN_RUNTIME_PROTOCOL_MAX_PAYLOAD) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "worker recovery snapshot has an invalid size");
        return -1;
    }
    int fd = memfd_create("gnoblin-runtime-recovery", MFD_CLOEXEC | MFD_ALLOW_SEALING);
    if (fd < 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "could not create worker recovery descriptor: %s", g_strerror(errno));
        return -1;
    }
    for (gsize written = 0; written < length;) {
        ssize_t result = write(fd, data + written, length - written);
        if (result > 0) {
            written += result;
            continue;
        }
        if (result < 0 && errno == EINTR)
            continue;
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "could not write worker recovery snapshot: %s", g_strerror(errno));
        close(fd);
        return -1;
    }
    if (lseek(fd, 0, SEEK_SET) < 0 ||
        fcntl(fd, F_ADD_SEALS, F_SEAL_SEAL | F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_WRITE) < 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "could not seal worker recovery snapshot: %s", g_strerror(errno));
        close(fd);
        return -1;
    }
    return fd;
}

GPid spawn_runtime_worker(const char* executable, const char* config_path, gboolean devkit,
                          const char* wayland_display, gboolean resume, gboolean send_autostart,
                          const char* startup_recovery, GVariant* recovery_snapshot,
                          guint64 settings_revision, guint64 runtime_generation,
                          guint64 operation_id_watermark, int channel_fd, int host_control_fd,
                          int guardian_status_fd, int* ready_fd, GError** error) {
    g_autofree char* ready_arg = NULL;
    g_autofree char* revision_arg = NULL;
    g_autofree char* generation_arg = NULL;
    g_autofree char* operation_id_watermark_arg = NULL;
    g_autofree char* host_control_arg = NULL;
    g_autoptr(GPtrArray) argv = NULL;
    g_auto(GStrv) worker_environment = NULL;
    int status_pipe[2] = {-1, -1};
    int channel_parent_alias = -1, channel_child_alias = -1;
    int status_parent_alias = -1, status_child_alias = -1;
    int host_parent_alias = -1, host_child_alias = -1, guardian_status_alias = -1;
    int recovery_snapshot_fd = -1, recovery_snapshot_alias = -1;
    if (recovery_snapshot &&
        (recovery_snapshot_fd = create_recovery_snapshot_fd(recovery_snapshot, error)) < 0)
        goto fail;
    if (pipe2(status_pipe, O_CLOEXEC | O_NONBLOCK) != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "could not create worker status pipe: %s", g_strerror(errno));
        if (recovery_snapshot_fd >= 0)
            close(recovery_snapshot_fd);
        return 0;
    }
    if (!move_fd_above_runtime_targets(channel_fd, &channel_parent_alias, error) ||
        !move_fd_above_runtime_targets(channel_fd, &channel_child_alias, error) ||
        !move_fd_above_runtime_targets(status_pipe[0], &status_parent_alias, error) ||
        !move_fd_above_runtime_targets(status_pipe[1], &status_child_alias, error) ||
        !move_fd_above_runtime_targets(guardian_status_fd, &guardian_status_alias, error) ||
        (recovery_snapshot_fd >= 0 &&
         !move_fd_above_runtime_targets(recovery_snapshot_fd, &recovery_snapshot_alias, error)) ||
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
    int recovery_action_result = 0;
    gboolean actions_ready =
        gnoblin_runtime_spawn_add_channel_actions(&actions, channel_parent_alias,
                                                  channel_child_alias, RUNTIME_FD,
                                                  &channel_temporary_fd, error) &&
        gnoblin_runtime_spawn_add_channel_actions(&actions, status_parent_alias, status_child_alias,
                                                  WORKER_READY_FD, &status_temporary_fd, error) &&
        (host_control_fd < 0 ||
         gnoblin_runtime_spawn_add_channel_actions(&actions, host_parent_alias, host_child_alias,
                                                   WORKER_HOST_FD, &host_temporary_fd, error));
    if (actions_ready && recovery_snapshot_alias >= 0) {
        recovery_action_result = posix_spawn_file_actions_adddup2(&actions, recovery_snapshot_alias,
                                                                  WORKER_RECOVERY_SNAPSHOT_FD);
        if (recovery_action_result == 0)
            recovery_action_result =
                posix_spawn_file_actions_addclose(&actions, recovery_snapshot_alias);
        if (recovery_action_result != 0) {
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(recovery_action_result),
                        "could not prepare worker recovery descriptor: %s",
                        g_strerror(recovery_action_result));
            actions_ready = FALSE;
        }
    }
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

    ready_arg = g_strdup_printf("%d", WORKER_READY_FD);
    revision_arg = g_strdup_printf("%" G_GUINT64_FORMAT, settings_revision);
    generation_arg = g_strdup_printf("%" G_GUINT64_FORMAT, runtime_generation);
    operation_id_watermark_arg = g_strdup_printf("%" G_GUINT64_FORMAT, operation_id_watermark);
    host_control_arg = g_strdup_printf("%d", WORKER_HOST_FD);
    argv = g_ptr_array_new_with_free_func(g_free);
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
    worker_environment = g_get_environ();
    worker_environment =
        g_environ_unsetenv(worker_environment, "GNOBLIN_INTERNAL_STARTUP_RECOVERY");
    if (startup_recovery)
        worker_environment = g_environ_setenv(
            worker_environment, "GNOBLIN_INTERNAL_STARTUP_RECOVERY", startup_recovery, TRUE);
    result = posix_spawn(&pid, executable, &actions, NULL, (char**)argv->pdata, worker_environment);
    posix_spawn_file_actions_destroy(&actions);
    if (channel_temporary_fd >= 0)
        close(channel_temporary_fd);
    if (status_temporary_fd >= 0)
        close(status_temporary_fd);
    if (host_temporary_fd >= 0)
        close(host_temporary_fd);
    if (guardian_status_alias >= 0)
        close(guardian_status_alias);
    if (recovery_snapshot_alias >= 0)
        close(recovery_snapshot_alias);
    if (recovery_snapshot_fd >= 0)
        close(recovery_snapshot_fd);
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
    if (recovery_snapshot_alias >= 0)
        close(recovery_snapshot_alias);
    if (recovery_snapshot_fd >= 0)
        close(recovery_snapshot_fd);
    close(status_pipe[0]);
    close(status_pipe[1]);
    return 0;
}

gboolean drain_worker_ready_fd(int fd, gboolean* start_packet_sent) {
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

static gboolean send_runtime_control_packet_with_payload(int channel_fd,
                                                         GnoblinRuntimePacketType type,
                                                         GVariant* payload,
                                                         const char* notification, GError** error) {
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    if (!gnoblin_runtime_writer_queue(writer, type, 0, payload, error))
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

static gboolean send_runtime_control_packet(int channel_fd, GnoblinRuntimePacketType type,
                                            const char* notification, GError** error) {
    g_autoptr(GVariant) empty =
        g_variant_ref_sink(g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
    return send_runtime_control_packet_with_payload(channel_fd, type, empty, notification, error);
}

static gboolean send_session_lifecycle_state(int channel_fd, const char* state, guint64 revision,
                                             GError** error) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "state", g_variant_new_string(state));
    g_variant_builder_add(&builder, "{sv}", "revision", g_variant_new_uint64(revision));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    return send_runtime_control_packet_with_payload(
        channel_fd, GNOBLIN_RUNTIME_PACKET_SESSION_STATE_CHANGED, payload,
        "Mutter did not accept the session lifecycle update", error);
}

gboolean wait_for_session_state_published(int channel_fd, guint64 revision) {
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    gint64 deadline = g_get_monotonic_time() + 500 * 1000;
    while (g_get_monotonic_time() < deadline) {
        gint64 remaining = deadline - g_get_monotonic_time();
        struct pollfd pfd = {.fd = channel_fd, .events = POLLIN | POLLHUP};
        int timeout = (int)MIN((remaining + 999) / 1000, 50);
        int result;
        do {
            result = poll(&pfd, 1, timeout);
        } while (result < 0 && errno == EINTR);
        if (result < 0 || (result > 0 && (pfd.revents & (POLLERR | POLLNVAL))))
            return FALSE;
        if (result == 0)
            continue;
        for (;;) {
            GnoblinRuntimePacket packet = {0};
            gboolean available = FALSE;
            g_autoptr(GError) error = NULL;
            if (!gnoblin_runtime_reader_receive(reader, channel_fd, &packet, &available, &error))
                return FALSE;
            if (!available)
                break;
            gboolean published = packet.type == GNOBLIN_RUNTIME_PACKET_SESSION_STATE_PUBLISHED &&
                                 packet.request_id == revision &&
                                 g_variant_n_children(packet.payload) == 0;
            gnoblin_runtime_packet_clear(&packet);
            if (published)
                return TRUE;
        }
        if (pfd.revents & POLLHUP)
            return FALSE;
    }
    return FALSE;
}

gboolean guardian_set_session_lifecycle_state(int channel_fd, const char** current_state,
                                              guint64* revision, const char* state) {
    if (!current_state || !revision || !state)
        return FALSE;
    if (g_strcmp0(*current_state, state) == 0)
        return TRUE;
    if (!g_str_equal(state, "starting") && !g_str_equal(state, "running") &&
        !g_str_equal(state, "stopping"))
        return FALSE;
    if (*revision == G_MAXUINT64) {
        g_printerr("gnoblin: session lifecycle revision overflow\n");
        return FALSE;
    }
    guint64 next_revision = *revision + 1;
    g_autoptr(GError) error = NULL;
    if (!send_session_lifecycle_state(channel_fd, state, next_revision, &error)) {
        g_printerr("gnoblin: could not publish session lifecycle state: %s\n",
                   error ? error->message : "notification failed");
        return FALSE;
    }
    *current_state = state;
    *revision = next_revision;
    return TRUE;
}

gboolean send_worker_disconnected(int channel_fd, GError** error) {
    return send_runtime_control_packet(channel_fd, GNOBLIN_RUNTIME_PACKET_WORKER_DISCONNECTED,
                                       "Mutter did not accept worker disconnect notification",
                                       error);
}

void notify_runtime_recovery_failed(int channel_fd) {
    append_config_notice("Lua runtime recovery failed. The compositor is still running, "
                         "but configuration and Lua callbacks are unavailable. "
                         "Log out and start a new session.");
    g_autoptr(GError) error = NULL;
    if (!send_runtime_control_packet(channel_fd, GNOBLIN_RUNTIME_PACKET_RECOVERY_FAILED,
                                     "Mutter did not accept runtime recovery failure notification",
                                     &error))
        g_printerr("gnoblin: could not report terminal runtime recovery failure: %s\n",
                   error ? error->message : "notification failed");
}

gboolean wait_for_worker_suspended(int channel_fd, Runtime* host, guint64* settings_revision,
                                   guint64* runtime_generation, guint64* operation_id_watermark,
                                   GVariant** snapshot, GError** error) {
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
                g_autoptr(GVariant) document =
                    g_variant_lookup_value(packet.payload, "document", G_VARIANT_TYPE_VARDICT);
                g_autoptr(GVariant) events =
                    g_variant_lookup_value(packet.payload, "runtime_events", G_VARIANT_TYPE("as"));
                gboolean valid =
                    packet.request_id == 0 &&
                    g_variant_lookup(packet.payload, "settings_revision", "t", settings_revision) &&
                    g_variant_lookup(packet.payload, "runtime_generation", "t",
                                     runtime_generation) &&
                    g_variant_lookup(packet.payload, "operation_id_watermark", "t",
                                     operation_id_watermark) &&
                    document && events && *settings_revision > 0 && *runtime_generation > 0 &&
                    *operation_id_watermark <= G_MAXINT64;
                if (valid && snapshot) {
                    g_clear_pointer(snapshot, g_variant_unref);
                    *snapshot = g_variant_ref(packet.payload);
                }
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

gboolean sleep_before_worker_restart(guint restart_number, Runtime* host) {
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
