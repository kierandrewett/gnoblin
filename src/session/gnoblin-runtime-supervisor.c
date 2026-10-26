#include "gnoblin-runtime-internal.h"

static gboolean perform_power_action(int exit_status) {
    static const struct {
        int exit_status;
        const char* method;
        const char* query;
        const char* label;
    } actions[] = {{EXIT_POWER_SUSPEND, "Suspend", "CanSuspend", "suspend"},
                   {EXIT_POWER_HIBERNATE, "Hibernate", "CanHibernate", "hibernate"},
                   {EXIT_POWER_REBOOT, "Reboot", "CanReboot", "restart"},
                   {EXIT_POWER_POWEROFF, "PowerOff", "CanPowerOff", "power off"}};
    const typeof(actions[0])* action = NULL;
    for (guint i = 0; i < G_N_ELEMENTS(actions); i++)
        if (actions[i].exit_status == exit_status) {
            action = &actions[i];
            break;
        }
    if (!action)
        return FALSE;

    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) bus = g_bus_get_sync(G_BUS_TYPE_SYSTEM, NULL, &error);
    if (!bus)
        goto failed;
    g_autoptr(GVariant) answer = g_dbus_connection_call_sync(
        bus, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager",
        action->query, NULL, G_VARIANT_TYPE("(s)"), G_DBUS_CALL_FLAGS_NONE, 5000, NULL, &error);
    if (!answer)
        goto failed;
    const char* verdict = NULL;
    g_variant_get(answer, "(&s)", &verdict);
    if (!g_str_equal(verdict, "yes") && !g_str_equal(verdict, "challenge")) {
        g_set_error(&error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                    "this machine cannot %s here (logind answers \"%s\")", action->label, verdict);
        goto failed;
    }
    g_autoptr(GVariant) result = g_dbus_connection_call_sync(
        bus, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager",
        action->method, g_variant_new("(b)", TRUE), NULL, G_DBUS_CALL_FLAGS_NONE, 30000, NULL,
        &error);
    if (result)
        return TRUE;
    if (error)
        g_dbus_error_strip_remote_error(error);
failed:
    g_printerr("gnoblin: session power action failed: %s\n",
               error ? error->message : "unknown logind error");
    return FALSE;
}

GPid spawn_session_supervisor(const char* executable, const char* config_path,
                              GnoblinNestedMode nested, const char* wayland_display, int runtime_fd,
                              int status_parent_fd, int status_child_fd, int autostart_parent_fd,
                              int autostart_child_fd, gboolean resume, guint64 settings_revision,
                              guint64 runtime_generation, guint64 operation_id_watermark,
                              gboolean autostart_complete, GVariant* recovery_snapshot,
                              GError** error) {
    g_autofree char* revision_arg = NULL;
    g_autofree char* generation_arg = NULL;
    g_autofree char* watermark_arg = NULL;
    g_autoptr(GPtrArray) argv = NULL;
    int runtime_alias = -1, status_parent_alias = -1, status_child_alias = -1;
    int autostart_parent_alias = -1, autostart_child_alias = -1;
    int recovery_snapshot_fd = -1, recovery_snapshot_alias = -1;
    if (recovery_snapshot &&
        (recovery_snapshot_fd = create_recovery_snapshot_fd(recovery_snapshot, error)) < 0)
        goto fail;
    if (!move_fd_above_runtime_targets(runtime_fd, &runtime_alias, error) ||
        !move_fd_above_runtime_targets(status_parent_fd, &status_parent_alias, error) ||
        !move_fd_above_runtime_targets(status_child_fd, &status_child_alias, error) ||
        !move_fd_above_runtime_targets(autostart_parent_fd, &autostart_parent_alias, error) ||
        !move_fd_above_runtime_targets(autostart_child_fd, &autostart_child_alias, error) ||
        (recovery_snapshot_fd >= 0 &&
         !move_fd_above_runtime_targets(recovery_snapshot_fd, &recovery_snapshot_alias, error)))
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
    gboolean actions_ready = runtime_result == 0;
    if (actions_ready && recovery_snapshot_alias >= 0) {
        int snapshot_result = posix_spawn_file_actions_adddup2(&actions, recovery_snapshot_alias,
                                                               WORKER_RECOVERY_SNAPSHOT_FD);
        if (snapshot_result == 0)
            snapshot_result = posix_spawn_file_actions_addclose(&actions, recovery_snapshot_alias);
        if (snapshot_result != 0) {
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(snapshot_result),
                        "could not prepare supervisor recovery descriptor: %s",
                        g_strerror(snapshot_result));
            actions_ready = FALSE;
        }
    }
    actions_ready =
        actions_ready &&
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

    revision_arg = g_strdup_printf("%" G_GUINT64_FORMAT, settings_revision);
    generation_arg = g_strdup_printf("%" G_GUINT64_FORMAT, runtime_generation);
    watermark_arg = g_strdup_printf("%" G_GUINT64_FORMAT, operation_id_watermark);
    argv = g_ptr_array_new_with_free_func(g_free);
    g_ptr_array_add(argv, g_strdup(executable));
    g_ptr_array_add(argv, g_strdup("--internal-session-supervisor"));
    g_ptr_array_add(argv, g_strdup("--config"));
    g_ptr_array_add(argv, g_strdup(config_path));
    if (nested != GNOBLIN_NESTED_NONE) {
        g_ptr_array_add(argv, g_strdup(nested_mode_option(nested)));
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
    if (recovery_snapshot_alias >= 0)
        close(recovery_snapshot_alias);
    if (recovery_snapshot_fd >= 0)
        close(recovery_snapshot_fd);
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
    if (recovery_snapshot_alias >= 0)
        close(recovery_snapshot_alias);
    if (recovery_snapshot_fd >= 0)
        close(recovery_snapshot_fd);
    return 0;
}

int session_supervisor_main(int argc, char** argv) {
    g_autofree char* config_path = NULL;
    const char* wayland_display = NULL;
    GnoblinNestedMode nested = GNOBLIN_NESTED_NONE;
    gboolean resume_supervisor = FALSE;
    gboolean autostart_complete = FALSE;
    guint64 supervisor_seed_revision = 0, supervisor_seed_generation = 0;
    guint64 supervisor_seed_operation_id_watermark = 0;
    for (int i = 1; i < argc; i++) {
        if (g_str_equal(argv[i], "--config") && i + 1 < argc)
            config_path = g_canonicalize_filename(argv[++i], NULL);
        else if (g_str_equal(argv[i], "--devkit") || g_str_equal(argv[i], "--hosted")) {
            if (!nested_mode_select(&nested, g_str_equal(argv[i], "--devkit")
                                                 ? GNOBLIN_NESTED_DEVKIT
                                                 : GNOBLIN_NESTED_HOSTED))
                return EXIT_FAILURE;
        } else if (g_str_equal(argv[i], "--wayland-display") && i + 1 < argc)
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
    if (!nested_mode_display_valid(nested, wayland_display)) {
        usage(argv[0]);
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
    Runtime host = {.channel_fd = RUNTIME_FD,
                    .session_log_fd = -1,
                    .compositor_stderr_fd = -1,
                    .exit_status = EXIT_FAILURE};
    g_autofree char* executable = g_file_read_link("/proc/self/exe", NULL);
    if (!executable) {
        g_printerr("gnoblin: could not locate installed executable\n");
        return EXIT_FAILURE;
    }

    g_autoptr(GVariant) recovery_snapshot = NULL;
    if (fcntl(WORKER_RECOVERY_SNAPSHOT_FD, F_GETFD) >= 0) {
        recovery_snapshot = read_recovery_snapshot(WORKER_RECOVERY_SNAPSHOT_FD, &error);
        close(WORKER_RECOVERY_SNAPSHOT_FD);
        if (!recovery_snapshot) {
            g_printerr("gnoblin: could not read worker recovery snapshot: %s\n",
                       error ? error->message : "invalid descriptor");
            return EXIT_FAILURE;
        }
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
    gint64 worker_spawned_us = g_get_monotonic_time();
    const char* startup_recovery = NULL;
    guint64 resume_revision = 0, resume_generation = 0;
    guint64 resume_operation_id_watermark = 0;
    g_autoptr(GVariant) resume_snapshot = NULL;
    /* Even after guardian recovery, the resumed worker must be able to replace
     * config-owned autostart once embedded defaults are committed. */
    int host_control_sockets[2] = {-1, -1};
    if (socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, host_control_sockets) != 0) {
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
        executable, config_path, nested, wayland_display, resume_supervisor, !autostart_complete,
        NULL, recovery_snapshot, supervisor_seed_revision, supervisor_seed_generation,
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
                if (host_autostart_received && host_autostart_started &&
                    !forward_autostart_to_guardian(GUARDIAN_AUTOSTART_FD, host_autostart_entries,
                                                   host_autostart_environment, &error)) {
                    g_printerr("gnoblin: could not update session autostart: %s\n",
                               error ? error->message : "could not forward snapshot");
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
            }
            /* The control and ready pipes are polled independently. A worker can
             * write its autostart snapshot between those polls, so never treat a
             * READY byte as proof that this iteration already read the snapshot. */
            if (worker_ready && host_autostart_entries && host_autostart_environment &&
                !host_autostart_started) {
                if (!host_autostart_entries || !host_autostart_environment ||
                    !forward_autostart_to_guardian(GUARDIAN_AUTOSTART_FD, host_autostart_entries,
                                                   host_autostart_environment, &error)) {
                    g_printerr("gnoblin: could not start login autostart: %s\n",
                               error ? error->message : "invalid autostart snapshot");
                    stop_session = TRUE;
                    break;
                }
                host_autostart_started = TRUE;
            }
            if (worker_ready && !host_autostart_entries && !host_autostart_started &&
                worker_ready_since_us &&
                g_get_monotonic_time() - worker_ready_since_us > STARTUP_TIMEOUT_MS * 1000) {
                g_printerr("gnoblin: Lua runtime became ready without an autostart snapshot; "
                           "restarting recovery worker\n");
                kill(worker_pid, SIGKILL);
            }
            if (worker_ready_since_us &&
                g_get_monotonic_time() - worker_ready_since_us >= WORKER_STABLE_RESET_MS * 1000)
                restart_count = 0;
            /* Lua evaluation happens before the worker's own GLib timeout
             * exists. Bound it here while native bootstrap waits for CONFIG. */
            if (!worker_start_packet_sent && !worker_ready &&
                g_get_monotonic_time() - worker_spawned_us > WORKER_CONFIG_TIMEOUT_MS * 1000) {
                startup_recovery = "Lua runtime did not produce a startup configuration in time";
                kill(worker_pid, SIGKILL);
            }
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
                if (WIFEXITED(worker_status) && WEXITSTATUS(worker_status) >= EXIT_POWER_SUSPEND &&
                    WEXITSTATUS(worker_status) <= EXIT_POWER_POWEROFF) {
                    host.exit_status = perform_power_action(WEXITSTATUS(worker_status))
                                           ? EXIT_SUCCESS
                                           : EXIT_FAILURE;
                    stop_session = TRUE;
                    break;
                }
                if (WIFEXITED(worker_status) && WEXITSTATUS(worker_status) == EXIT_CONFIG_RESTORE) {
                    if (!gnoblin_config_restore_default(&error)) {
                        g_printerr("gnoblin: could not restore default config: %s\n",
                                   error ? error->message : "restore failed");
                        g_clear_error(&error);
                    }
                    /* Use the normal supervised worker recovery path to load the restored tree. */
                }
                if (WIFEXITED(worker_status) &&
                    WEXITSTATUS(worker_status) == EXIT_RESUME_REJECTED) {
                    g_printerr("gnoblin: Mutter rejected runtime resume; keeping the session host "
                               "and compositor alive\n");
                    notify_runtime_recovery_failed(host.channel_fd);
                    leave_session_alive = TRUE;
                    break;
                }
                gboolean bootstrap_retry = first_worker && !worker_start_packet_sent &&
                                           !worker_ready && !resume_supervisor;
                if (bootstrap_retry) {
                    if (!startup_recovery)
                        startup_recovery =
                            "Lua runtime exited before producing a startup configuration";
                    set_config_fallback_marker(TRUE, FALSE, startup_recovery);
                    g_printerr("gnoblin: %s; retrying with embedded defaults\n", startup_recovery);
                } else {
                    first_worker = FALSE;
                }
                restart_count++;
                /* If a replacement failed before sending RESUME, Mutter is
                 * still in the SUSPENDED state already acknowledged above. */
                if (worker_start_packet_sent) {
                    g_autoptr(GError) suspend_error = NULL;
                    if (!send_worker_disconnected(host.channel_fd, &suspend_error) ||
                        !wait_for_worker_suspended(
                            host.channel_fd, &host, &resume_revision, &resume_generation,
                            &resume_operation_id_watermark, &resume_snapshot, &suspend_error)) {
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
                /* A failed initial worker never completed HELLO. It needs a
                 * fresh CONFIG and autostart channel, not a WORKER_RESUME. */
                {
                    if (host_control_fd >= 0)
                        close(host_control_fd);
                    host_control_fd = -1;
                    gnoblin_runtime_reader_free(g_steal_pointer(&host_control_reader));
                    host_control_reader = gnoblin_runtime_reader_new();
                    host_autostart_received = FALSE;
                    g_clear_pointer(&host_autostart_entries, g_variant_unref);
                    g_clear_pointer(&host_autostart_environment, g_variant_unref);
                    if (socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0,
                                   host_control_sockets) != 0) {
                        g_printerr("gnoblin: could not recreate runtime supervisor channel: %s\n",
                                   g_strerror(errno));
                        stop_session = TRUE;
                        break;
                    }
                    host_control_fd = host_control_sockets[0];
                }
                g_clear_error(&error);
                worker_pid = spawn_runtime_worker(
                    executable, config_path, nested, wayland_display, !bootstrap_retry, TRUE,
                    startup_recovery, resume_snapshot, resume_revision, resume_generation,
                    resume_operation_id_watermark, host.channel_fd, host_control_sockets[1],
                    GUARDIAN_STATUS_FD, &ready_fd, &error);
                worker_spawned_us = g_get_monotonic_time();
                if (host_control_sockets[1] >= 0) {
                    close(host_control_sockets[1]);
                    host_control_sockets[1] = -1;
                }
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
