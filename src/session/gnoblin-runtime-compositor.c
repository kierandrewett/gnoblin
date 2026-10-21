#include "gnoblin-runtime-internal.h"

static gboolean handle_runtime_packet(Runtime* runtime, GnoblinRuntimePacket* packet,
                                      GError** error);

/* User code is evaluated only after the safe compositor has completed HELLO.
 * Native settings use the same rollback transaction as an explicit reload. */
gboolean runtime_load_startup_config(gpointer user_data) {
    Runtime* runtime = user_data;
    runtime->startup_config_idle_id = 0;
    if ((!runtime->startup_config_pending && !runtime->startup_saved_pending) || runtime->failed)
        return G_SOURCE_REMOVE;
    gboolean trying_saved = runtime->startup_saved_pending;
    GVariantBuilder arguments;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    GVariantBuilder request;
    g_variant_builder_init(&request, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&request, "{sv}", "kind", g_variant_new_string("reload"));
    g_variant_builder_add(&request, "{sv}", "method",
                          g_variant_new_string("runtime.reload_config"));
    g_variant_builder_add(&request, "{sv}", "arguments", g_variant_builder_end(&arguments));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&request));
    g_autoptr(GError) error = NULL;
    if (!handle_api_request(runtime, G_MAXUINT64, payload, &error))
        runtime_fail(runtime, error ? error->message : "could not stage startup configuration");
    runtime->startup_config_pending = FALSE;
    runtime->startup_saved_pending = FALSE;
    if (!trying_saved && !runtime->pending_reload && runtime->startup_failure) {
        runtime->startup_saved_pending = TRUE;
        runtime->startup_config_idle_id = g_idle_add(runtime_load_startup_config, runtime);
    }
    return G_SOURCE_REMOVE;
}

gboolean startup_timeout(gpointer user_data) {
    Runtime* runtime = user_data;
    if (!runtime->ready)
        runtime_fail(runtime, "compositor did not complete the private runtime handshake");
    runtime->startup_timeout_id = 0;
    return G_SOURCE_REMOVE;
}

gboolean runtime_signal_received(gpointer user_data) {
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
    if (packet->type == GNOBLIN_RUNTIME_PACKET_INPUT && packet->request_id > 0) {
        handled = handle_input(runtime, packet->request_id, packet->payload, error);
    } else if (packet->type == GNOBLIN_RUNTIME_PACKET_ERROR) {
        const char* message = NULL;
        gboolean resume_rejected = FALSE;
        g_variant_lookup(packet->payload, "message", "&s", &message);
        g_variant_lookup(packet->payload, "resume_rejected", "b", &resume_rejected);
        runtime->resume_rejected = runtime->resume_worker && resume_rejected;
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "compositor rejected runtime: %s",
                    message ? message : "unspecified error");
    } else if (!runtime->ready) {
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
            guint32 session_lifecycle_version = 0;
            if (runtime->guardian_status_fd >= 0 &&
                g_variant_lookup(packet->payload, "session_lifecycle_version", "u",
                                 &session_lifecycle_version) &&
                session_lifecycle_version == 1 &&
                !guardian_send_status(runtime->guardian_status_fd,
                                      GUARDIAN_STATUS_SESSION_LIFECYCLE_SUPPORTED)) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                                    "could not report session lifecycle protocol support");
                return FALSE;
            }
            g_clear_pointer(&runtime->session_environment, g_variant_unref);
            runtime->session_environment =
                g_variant_lookup_value(packet->payload, "environment", G_VARIANT_TYPE("a{ss}"));
            if (!runtime->session_environment) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "Mutter HELLO did not provide the session environment");
                return FALSE;
            }
            if (runtime->send_initial_autostart &&
                !send_autostart_snapshot(runtime->host_control_fd, runtime->initial_document,
                                         runtime->session_environment, error))
                return FALSE;
            runtime->ready = TRUE;
            g_autofree char* config_path = gnoblin_config_path();
            g_autoptr(GVariant) accepted_document = gnoblin_config_current_document();
            g_autoptr(GError) save_error = NULL;
            if (!runtime->startup_config_pending && !runtime->config_fallback &&
                accepted_document &&
                !gnoblin_config_save_last_good_document(config_path, accepted_document,
                                                        &save_error))
                g_warning("gnoblin: could not save last accepted configuration: %s",
                          save_error ? save_error->message : "unknown error");
            if (!runtime->startup_config_pending)
                set_config_fallback_marker(
                    runtime->config_fallback && runtime->config_fallback_error != NULL,
                    runtime->config_fallback_last_good, runtime->config_fallback_error);
            if (runtime->guardian_status_fd >= 0)
                (void)guardian_send_status(runtime->guardian_status_fd, GUARDIAN_STATUS_READY);
            /* Recovery keeps the host waiting for the first default CONFIG and
             * its replacement autostart snapshot. */
            if (!runtime->recovery_snapshot)
                runtime_signal_ready(runtime);
            if (runtime->startup_timeout_id) {
                g_source_remove(runtime->startup_timeout_id);
                runtime->startup_timeout_id = 0;
            }
            runtime_schedule_deferred_packets(runtime);
            if (runtime->startup_config_pending)
                runtime->startup_config_idle_id = g_idle_add(runtime_load_startup_config, runtime);
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

void runtime_schedule_deferred_packets(Runtime* runtime) {
    if (runtime && !runtime->pending_reload && !runtime->deferred_packets_idle_id &&
        !g_queue_is_empty(&runtime->deferred_packets))
        runtime->deferred_packets_idle_id = g_idle_add_full(
            G_PRIORITY_DEFAULT_IDLE, runtime_dispatch_deferred_packets, runtime, NULL);
}

gboolean runtime_channel_ready(gint fd, GIOCondition condition, gpointer user_data) {
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
            if (runtime->recovery_snapshot) {
                gnoblin_runtime_packet_clear(&packet);
                continue;
            }
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
            if (runtime->recovery_snapshot && packet.type == GNOBLIN_RUNTIME_PACKET_EVENT) {
                gnoblin_runtime_packet_clear(&packet);
                continue;
            }
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

void terminate_and_reap(Runtime* runtime) {
    if (!runtime || !runtime->compositor_pid)
        return;
    if (runtime->child_watch_id) {
        g_source_remove(runtime->child_watch_id);
        runtime->child_watch_id = 0;
    }
    GPid pid = runtime->compositor_pid;
    runtime->compositor_pid = 0;
    int status = 0;
    gboolean reaped = FALSE;
    // A compositor that is already gone did not stop because of our signal.
    // Say how it ended, so a crash shows in the log and not only as a closed channel.
    if (waitpid(pid, &status, WNOHANG) == pid) {
        if (WIFSIGNALED(status))
            g_printerr("gnoblin: runtime supervisor: the compositor ended on signal %d\n",
                       WTERMSIG(status));
        else if (WIFEXITED(status))
            g_printerr("gnoblin: runtime supervisor: the compositor exited with status %d\n",
                       WEXITSTATUS(status));
        g_spawn_close_pid(pid);
        return;
    }
    kill(pid, SIGTERM);
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
gboolean host_reap_compositor(Runtime* runtime) {
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
        session_diagnostic_wait_status(runtime, "compositor", pid, status);
        record_compositor_crash(WIFSIGNALED(status) ? WTERMSIG(status) : 0,
                                WIFEXITED(status) ? WEXITSTATUS(status) : 0);
        runtime->exit_status = WIFEXITED(status)     ? WEXITSTATUS(status)
                               : WIFSIGNALED(status) ? 128 + WTERMSIG(status)
                                                     : EXIT_FAILURE;
    } else {
        runtime->exit_status = EXIT_FAILURE;
    }
    g_spawn_close_pid(pid);
    return TRUE;
}

GPid spawn_compositor(const char* path, int parent_fd, int child_fd, int stderr_fd,
                      gboolean xwayland, gboolean devkit, const char* wayland_display,
                      GError** error) {
    /* This private capability marker makes packet 17 opt-in across mixed
     * Gnoblin/Mutter installations. Do not leak it to applications spawned
     * by the session. */
    g_auto(GStrv) compositor_environment =
        g_environ_setenv(g_get_environ(), "GNOBLIN_SESSION_LIFECYCLE_VERSION", "1", TRUE);
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
    if (stderr_fd >= 0) {
        result = posix_spawn_file_actions_adddup2(&actions, stderr_fd, STDERR_FILENO);
        if (result == 0 && stderr_fd != STDERR_FILENO)
            result = posix_spawn_file_actions_addclose(&actions, stderr_fd);
        if (result != 0) {
            posix_spawn_file_actions_destroy(&actions);
            if (temporary_fd >= 0)
                close(temporary_fd);
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(result),
                        "could not capture compositor stderr: %s", g_strerror(result));
            return 0;
        }
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
    g_ptr_array_add(argv, g_steal_pointer(&fd_option));
    if (!xwayland)
        g_ptr_array_add(argv, g_strdup("--no-x11"));
    g_ptr_array_add(argv, NULL);

    pid_t pid = 0;
    result = posix_spawn(&pid, path, &actions, NULL, (char**)argv->pdata, compositor_environment);
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

void usage(const char* program) {
    g_printerr("Usage: %s [--config PATH] [--xwayland|--no-xwayland] [--greeter] "
               "[--devkit --wayland-display NAME]\n",
               program);
}
