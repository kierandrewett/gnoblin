#include "gnoblin-runtime-internal.h"

static gboolean guardian_drain_status(int fd, gboolean* native_channel_started,
                                      gboolean* supervisor_ready, gboolean* session_ready,
                                      gint64* ready_since_us, gboolean* explicit_logout,
                                      gboolean* session_lifecycle_supported, GError** error) {
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
            if (!*session_ready)
                report_previous_crash();
            *session_ready = TRUE;
            break;
        case GUARDIAN_STATUS_LOGOUT:
            *explicit_logout = TRUE;
            break;
        case GUARDIAN_STATUS_SESSION_LIFECYCLE_SUPPORTED:
            *session_lifecycle_supported = TRUE;
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
                                           GPtrArray** active_entries, GError** error) {
    if (!gnoblin_autostart_receive_packet(fd, reader, received, entries, environment, error)) {
        if (!*received && error && *error &&
            g_error_matches(*error, G_IO_ERROR, G_IO_ERROR_CLOSED)) {
            g_clear_error(error);
            return TRUE;
        }
        return FALSE;
    }
    if (*received) {
        if (!*autostart_complete) {
            /* XDG autostart remains a login action. Gnoblin config autostart
             * below is reconciled for every accepted configuration snapshot. */
            *autostart_complete = TRUE;
            start_xdg_autostart(*environment);
        }
        /* reconcile_autostart checks the whole list before it stops or starts a process, so a
         * rejected list leaves the previous entries running. A bad entry in the config must not end
         * the user's session. */
        if (!reconcile_autostart(*entries, *environment, children, active_entries, error)) {
            const char* reason = error && *error ? (*error)->message : "invalid autostart data";
            g_autofree char* diagnostic = g_strdup_printf("Autostart config ignored: %s", reason);
            g_printerr("gnoblin: %s\n", diagnostic);
            append_config_notice(diagnostic);
            g_clear_error(error);
        }
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
    g_autofree char* config_path = NULL;
    const char* wayland_display = NULL;
    g_autofree char* xwayland_program = g_find_program_in_path("Xwayland");
    gboolean xwayland = xwayland_program != NULL;
    gboolean devkit = FALSE;
    Runtime guardian = {.channel_fd = -1,
                        .compositor_stderr_fd = -1,
                        .session_log_fd = -1,
                        .exit_status = EXIT_FAILURE};
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
    gnoblin_restore_interactive_priority("guardian");
    rotate_session_diagnostic("session-last.log", "session-previous.log");
    rotate_session_diagnostic("compositor-last.log", "compositor-previous.log");
    guardian.session_log_fd = open_session_diagnostic("session-last.log");
    int compositor_log_fd = open_session_diagnostic("compositor-last.log");
    session_diagnostic_event(&guardian, "guardian started");
    guardian_started_us = g_get_monotonic_time();
    discard_stale_crash_notice();
    if (!devkit && !prepare_session_environment()) {
        session_diagnostic_event(&guardian, "prepare-session-environment failed");
        if (compositor_log_fd >= 0)
            close(compositor_log_fd);
        if (guardian.session_log_fd >= 0)
            close(guardian.session_log_fd);
        return EXIT_FAILURE;
    }
    if (!config_path)
        config_path = gnoblin_config_path();
    g_setenv("GNOBLIN_CONFIG", config_path, TRUE);
    /* The guardian must not execute user Lua. The worker applies user
     * portal routes only after the configuration transaction is accepted. */
    g_autoptr(GVariant) initial_document = gnoblin_config_default_document();
    g_autoptr(GError) portal_error = NULL;
    if (!gnoblin_config_sync_portal_selection(initial_document, NULL, &portal_error))
        g_warning("gnoblin: could not update portal selection from Lua config: %s",
                  portal_error ? portal_error->message : "unknown error");
    if (!g_file_test(compositor_path, G_FILE_TEST_IS_EXECUTABLE)) {
        session_diagnostic_event(&guardian, "compositor executable unavailable");
        g_printerr("gnoblin: installed compositor is unavailable: %s\n", compositor_path);
        if (compositor_log_fd >= 0)
            close(compositor_log_fd);
        if (guardian.session_log_fd >= 0)
            close(guardian.session_log_fd);
        return EXIT_FAILURE;
    }
    if (!devkit && !activate_session()) {
        session_diagnostic_event(&guardian, "activate-session failed");
        if (compositor_log_fd >= 0)
            close(compositor_log_fd);
        if (guardian.session_log_fd >= 0)
            close(guardian.session_log_fd);
        return EXIT_FAILURE;
    }

    /* The guardian owns the lifecycle name even while supervisors restart. */
    g_autoptr(GDBusConnection) lifecycle_bus = claim_session_lifecycle_bus();
    struct sigaction action = {.sa_handler = host_signal_handler};
    sigemptyset(&action.sa_mask);
    sigaction(SIGTERM, &action, NULL);
    sigaction(SIGINT, &action, NULL);

    int sockets[2] = {-1, -1};
    if (socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets) != 0) {
        session_diagnostic_event(&guardian, "private-compositor-channel creation failed");
        g_printerr("gnoblin: could not create private compositor channel: %s\n", g_strerror(errno));
        if (compositor_log_fd >= 0)
            close(compositor_log_fd);
        if (guardian.session_log_fd >= 0)
            close(guardian.session_log_fd);
        return EXIT_FAILURE;
    }
    int compositor_stderr[2] = {-1, -1};
    if (pipe2(compositor_stderr, O_CLOEXEC) != 0) {
        session_diagnostic_event(&guardian, "compositor-stderr-pipe creation failed");
        close(sockets[0]);
        close(sockets[1]);
        if (compositor_log_fd >= 0)
            close(compositor_log_fd);
        if (guardian.session_log_fd >= 0)
            close(guardian.session_log_fd);
        return EXIT_FAILURE;
    }
    int flags = fcntl(compositor_stderr[0], F_GETFL);
    if (flags >= 0)
        (void)fcntl(compositor_stderr[0], F_SETFL, flags | O_NONBLOCK);
    g_autoptr(GError) error = NULL;
    guardian.channel_fd = sockets[0];
    guardian.compositor_stderr_fd = compositor_stderr[0];
    guardian.compositor_pid =
        spawn_compositor(compositor_path, sockets[0], sockets[1], compositor_stderr[1], xwayland,
                         devkit, wayland_display, &error);
    close(sockets[1]);
    close(compositor_stderr[1]);
    if (!guardian.compositor_pid) {
        close(sockets[0]);
        close(guardian.compositor_stderr_fd);
        session_diagnostic_event(&guardian, "compositor spawn failed");
        g_printerr("gnoblin: %s\n", error ? error->message : "compositor spawn failed");
        if (compositor_log_fd >= 0)
            close(compositor_log_fd);
        if (guardian.session_log_fd >= 0)
            close(guardian.session_log_fd);
        return EXIT_FAILURE;
    }
    session_diagnostic_event(&guardian, "compositor spawned pid=%d", (int)guardian.compositor_pid);
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
    g_autoptr(GPtrArray) active_autostart_entries = NULL;
    gboolean autostart_received = FALSE;
    gboolean autostart_complete = FALSE;
    gboolean native_channel_started = FALSE;
    gboolean supervisor_ready = FALSE;
    gboolean session_ready = FALSE;
    gboolean session_lifecycle_supported = FALSE;
    const char* session_lifecycle_state = NULL;
    guint64 session_lifecycle_revision = 0;
    gboolean explicit_logout = FALSE;
    gboolean stop_session = FALSE;
    gboolean retain_session = FALSE;
    gboolean resume_supervisor = FALSE;
    guint restart_count = 0;
    gint64 supervisor_ready_since_us = 0;
    guint64 resume_revision = 0, resume_generation = 0, resume_operation_id_watermark = 0;
    g_autoptr(GVariant) resume_snapshot = NULL;

    while (!host_signal_number && !stop_session) {
        drain_compositor_stderr(&guardian, compositor_log_fd);
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
                resume_operation_id_watermark, autostart_complete, resume_snapshot, &error);
            close(status_sockets[1]);
            status_sockets[1] = -1;
            close(autostart_sockets[1]);
            autostart_sockets[1] = -1;
            if (!supervisor_pid) {
                session_diagnostic_event(&guardian, "session supervisor spawn failed");
                g_printerr("gnoblin: could not start session supervisor: %s\n",
                           error ? error->message : "spawn failed");
                guardian.exit_status = EXIT_FAILURE;
                stop_session = TRUE;
                break;
            }
            session_diagnostic_event(&guardian, "session supervisor spawned pid=%d",
                                     (int)supervisor_pid);
            error = NULL;
        }

        struct pollfd polls[2] = {{.fd = status_fd, .events = POLLIN | POLLHUP},
                                  {.fd = autostart_fd, .events = POLLIN | POLLHUP}};
        (void)poll(polls, G_N_ELEMENTS(polls), 50);
        if (!guardian_drain_status(status_fd, &native_channel_started, &supervisor_ready,
                                   &session_ready, &supervisor_ready_since_us, &explicit_logout,
                                   &session_lifecycle_supported, &error)) {
            g_printerr("gnoblin: session supervisor status failed: %s\n",
                       error ? error->message : "invalid message");
            stop_session = TRUE;
            break;
        }
        if (session_lifecycle_supported && !session_lifecycle_state &&
            !guardian_set_session_lifecycle_state(guardian.channel_fd, &session_lifecycle_state,
                                                  &session_lifecycle_revision, "starting"))
            session_lifecycle_supported = FALSE;
        if (session_lifecycle_supported && supervisor_ready &&
            g_strcmp0(session_lifecycle_state, "starting") == 0 &&
            !guardian_set_session_lifecycle_state(guardian.channel_fd, &session_lifecycle_state,
                                                  &session_lifecycle_revision, "running"))
            session_lifecycle_supported = FALSE;
        if (!guardian_receive_autostart(autostart_fd, autostart_reader, &autostart_received,
                                        &autostart_complete, &autostart_entries,
                                        &autostart_environment, autostart_children,
                                        &active_autostart_entries, &error)) {
            g_printerr("gnoblin: login autostart data failed: %s\n",
                       error ? error->message : "invalid message");
            stop_session = TRUE;
            break;
        }
        if (host_reap_compositor(&guardian)) {
            drain_compositor_stderr(&guardian, compositor_log_fd);
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
        session_diagnostic_wait_status(&guardian, "session supervisor", supervisor_result,
                                       supervisor_status);
        kill(-supervisor_result, SIGKILL);
        if (!guardian_drain_status(status_fd, &native_channel_started, &supervisor_ready,
                                   &session_ready, &supervisor_ready_since_us, &explicit_logout,
                                   &session_lifecycle_supported, &error) ||
            !guardian_receive_autostart(autostart_fd, autostart_reader, &autostart_received,
                                        &autostart_complete, &autostart_entries,
                                        &autostart_environment, autostart_children,
                                        &active_autostart_entries, &error)) {
            g_printerr("gnoblin: could not collect final supervisor state: %s\n",
                       error ? error->message : "invalid message");
            stop_session = TRUE;
        }
        if (session_lifecycle_supported && !session_lifecycle_state &&
            !guardian_set_session_lifecycle_state(guardian.channel_fd, &session_lifecycle_state,
                                                  &session_lifecycle_revision, "starting"))
            session_lifecycle_supported = FALSE;
        if (session_lifecycle_supported && supervisor_ready &&
            g_strcmp0(session_lifecycle_state, "starting") == 0 &&
            !guardian_set_session_lifecycle_state(guardian.channel_fd, &session_lifecycle_state,
                                                  &session_lifecycle_revision, "running"))
            session_lifecycle_supported = FALSE;
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
            drain_compositor_stderr(&guardian, compositor_log_fd);
            stop_session = TRUE;
            break;
        }

        if (native_channel_started) {
            g_autoptr(GError) suspend_error = NULL;
            if (!send_worker_disconnected(guardian.channel_fd, &suspend_error) ||
                !wait_for_worker_suspended(guardian.channel_fd, &guardian, &resume_revision,
                                           &resume_generation, &resume_operation_id_watermark,
                                           &resume_snapshot, &suspend_error)) {
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
    stop_autostart_children(autostart_children);
    if (stop_session && session_ready)
        notify_portal_session_ending(lifecycle_bus);
    reap_autostart_children(autostart_children);
    if (stop_session && guardian.compositor_pid) {
        if (session_lifecycle_supported &&
            guardian_set_session_lifecycle_state(guardian.channel_fd, &session_lifecycle_state,
                                                 &session_lifecycle_revision, "stopping") &&
            !wait_for_session_state_published(guardian.channel_fd, session_lifecycle_revision))
            g_printerr(
                "gnoblin: Mutter did not confirm the session stopping event before shutdown\n");
        terminate_and_reap(&guardian);
    }
    drain_compositor_stderr(&guardian, compositor_log_fd);
    if (guardian.compositor_stderr_fd >= 0)
        close(guardian.compositor_stderr_fd);
    if (compositor_log_fd >= 0)
        close(compositor_log_fd);
    session_diagnostic_event(&guardian, "guardian exiting status=%d", guardian.exit_status);
    if (guardian.session_log_fd >= 0)
        close(guardian.session_log_fd);
    close(guardian.channel_fd);
    return guardian.exit_status;
}

int gnoblin_runtime_main(int argc, char** argv) {
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
