#include "gnoblin-runtime-internal.h"

/* Login managers do not consistently preserve a session's stderr after it
 * returns to the greeter. Keep a small, private record of process boundaries
 * and the compositor's own stderr. This deliberately never records the Lua
 * document, configuration paths, or environment. */
static char* session_diagnostic_path(const char* name) {
    return g_build_filename(g_get_user_state_dir(), "gnoblin", name, NULL);
}

void rotate_session_diagnostic(const char* current, const char* previous) {
    g_autofree char* current_path = session_diagnostic_path(current);
    g_autofree char* previous_path = session_diagnostic_path(previous);
    g_autofree char* directory = g_path_get_dirname(current_path);
    if (g_mkdir_with_parents(directory, 0700) == 0) {
        (void)g_unlink(previous_path);
        if (g_rename(current_path, previous_path) != 0 && errno != ENOENT)
            g_warning("gnoblin: could not rotate session diagnostic: %s", g_strerror(errno));
    }
}

int open_session_diagnostic(const char* name) {
    g_autofree char* path = session_diagnostic_path(name);
    g_autofree char* directory = g_path_get_dirname(path);
    if (g_mkdir_with_parents(directory, 0700) != 0)
        return -1;
    int fd = g_open(path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC | O_NOFOLLOW, 0600);
    if (fd >= 0)
        (void)fchmod(fd, 0600);
    return fd;
}

static void session_diagnostic_write(int fd, const char* text, gsize length) {
    if (fd < 0 || !text || !length)
        return;
    struct stat status;
    if (fstat(fd, &status) != 0 || status.st_size >= SESSION_DIAGNOSTIC_MAX_BYTES)
        return;
    length = MIN(length, (gsize)(SESSION_DIAGNOSTIC_MAX_BYTES - status.st_size));
    while (length) {
        ssize_t written = write(fd, text, length);
        if (written > 0) {
            text += written;
            length -= written;
        } else if (written < 0 && errno == EINTR) {
            continue;
        } else {
            break;
        }
    }
}

void session_diagnostic_event(Runtime* runtime, const char* format, ...) {
    if (!runtime || runtime->session_log_fd < 0)
        return;
    va_list arguments;
    va_start(arguments, format);
    g_autofree char* detail = g_strdup_vprintf(format, arguments);
    va_end(arguments);
    GDateTime* now = g_date_time_new_now_local();
    g_autofree char* stamp = g_date_time_format(now, "%FT%T%z");
    g_date_time_unref(now);
    g_autofree char* line = g_strdup_printf("%s %s\n", stamp, detail);
    session_diagnostic_write(runtime->session_log_fd, line, strlen(line));
}

void session_diagnostic_wait_status(Runtime* runtime, const char* role, GPid pid, int status) {
    if (WIFEXITED(status))
        session_diagnostic_event(runtime, "%s pid=%d exited status=%d", role, (int)pid,
                                 WEXITSTATUS(status));
    else if (WIFSIGNALED(status))
        session_diagnostic_event(runtime, "%s pid=%d killed signal=%d", role, (int)pid,
                                 WTERMSIG(status));
    else
        session_diagnostic_event(runtime, "%s pid=%d exited unrecognised-status=%d", role, (int)pid,
                                 status);
}

void drain_compositor_stderr(Runtime* runtime, int log_fd) {
    if (!runtime || runtime->compositor_stderr_fd < 0)
        return;
    char buffer[4096];
    gsize drained = 0;
    while (drained < SESSION_DIAGNOSTIC_MAX_BYTES) {
        ssize_t count = read(runtime->compositor_stderr_fd, buffer, sizeof buffer);
        if (count > 0) {
            session_diagnostic_write(log_fd, buffer, (gsize)count);
            /* Preserve the login manager's existing stderr routing too. */
            (void)write(STDERR_FILENO, buffer, (gsize)count);
            drained += count;
            continue;
        }
        if (count < 0 && errno == EINTR)
            continue;
        if (count == 0 || (count < 0 && errno != EAGAIN && errno != EWOULDBLOCK)) {
            close(runtime->compositor_stderr_fd);
            runtime->compositor_stderr_fd = -1;
        }
        return;
    }
}

static char* config_fallback_marker_path(void) {
    return g_build_filename(g_get_user_runtime_dir(), "gnoblin", "config-fallback", NULL);
}

char* config_recovery_notice(void) {
    g_autofree char* path = config_fallback_marker_path();
    g_autofree char* contents = NULL;
    gsize length = 0;
    if (!g_file_get_contents(path, &contents, &length, NULL) || !contents || length > 16384)
        return NULL;
    char* newline = strchr(contents, '\n');
    if (!newline || !newline[1])
        return NULL;
    char* diagnostic = g_strdup(newline + 1);
    g_strchomp(diagnostic);
    return diagnostic;
}

static void set_config_notice(const char* state, const char* diagnostic) {
    g_autofree char* path = config_fallback_marker_path();
    if (!state) {
        /* A crash notice describes the previous session, not this configuration,
         * so a clean configuration load must not clear it. */
        g_autofree char* existing = NULL;
        if (g_file_get_contents(path, &existing, NULL, NULL) &&
            g_str_has_prefix(existing, "crash\n"))
            return;
        if (g_unlink(path) != 0 && errno != ENOENT)
            g_warning("gnoblin: could not clear the configuration recovery notice: %s",
                      g_strerror(errno));
        return;
    }
    g_autofree char* directory = g_path_get_dirname(path);
    g_autofree char* contents =
        g_strdup_printf("%s\n%s\n", state, diagnostic ? diagnostic : "Configuration failed");
    if (g_mkdir_with_parents(directory, 0700) != 0 ||
        !g_file_set_contents_full(path, contents, -1, G_FILE_SET_CONTENTS_CONSISTENT, 0600, NULL))
        g_warning("gnoblin: could not publish the configuration recovery notice");
}

/* Keep the configuration diagnostic when a configured shell component also
 * fails. Bound accumulated notices and avoid reopening identical errors. */
void append_config_notice(const char* diagnostic) {
    g_autofree char* path = config_fallback_marker_path();
    g_autofree char* contents = NULL;
    gsize length = 0;
    if (g_file_get_contents(path, &contents, &length, NULL) && length <= 16384) {
        char* newline = strchr(contents, '\n');
        if (newline) {
            *newline = '\0';
            const char* previous = newline + 1;
            if (strstr(previous, diagnostic))
                return;
            if (g_str_equal(contents, "defaults") || g_str_equal(contents, "last-good") ||
                g_str_equal(contents, "notice")) {
                g_autofree char* combined = g_strdup_printf("%s\n%s", previous, diagnostic);
                set_config_notice(contents, combined);
                return;
            }
        }
    }
    set_config_notice("notice", diagnostic);
}

void set_config_fallback_marker(gboolean enabled, gboolean last_good, const char* diagnostic) {
    if (enabled && diagnostic && *diagnostic) {
        /* The panel reads the marker file. Also write the error and the choice to the session log,
         * as the documentation says, so that someone without the panel can find them. Every
         * fallback path comes through here and a reload sets the same marker again, so write each
         * distinct message once. */
        static char* last_logged = NULL;
        g_autofree char* line = g_strdup_printf(
            "gnoblin: configuration fell back to %s: %s",
            last_good ? "the last good configuration" : "the built-in defaults", diagnostic);
        if (g_strcmp0(line, last_logged)) {
            g_printerr("%s\n", line);
            g_free(last_logged);
            last_logged = g_steal_pointer(&line);
        }
    }
    set_config_notice(!enabled ? NULL : last_good ? "last-good" : "defaults", diagnostic);
}

/* A compositor that dies on a signal takes every client with it, and GDM then
 * shows the greeter with no explanation. Keep a short record in the state
 * directory so the next session can say what happened. SIGHUP, SIGINT and
 * SIGTERM are orderly stop requests, not crashes. */
gint64 guardian_started_us;

static char* crash_record_path(void) {
    return g_build_filename(g_get_user_state_dir(), "gnoblin", "crash-last", NULL);
}

void record_compositor_crash(int signal_number, int exit_status) {
    if (signal_number == SIGHUP || signal_number == SIGINT || signal_number == SIGTERM)
        return;
    if (!signal_number && !exit_status)
        return;
    g_autofree char* path = crash_record_path();
    g_autofree char* directory = g_path_get_dirname(path);
    const gint64 uptime =
        guardian_started_us ? (g_get_monotonic_time() - guardian_started_us) / G_USEC_PER_SEC : 0;
    g_autofree char* contents = g_strdup_printf(
        "signal=%d\nstatus=%d\nuptime=%" G_GINT64_FORMAT "\n", signal_number, exit_status, uptime);
    if (g_mkdir_with_parents(directory, 0700) != 0 ||
        !g_file_set_contents_full(path, contents, -1, G_FILE_SET_CONTENTS_CONSISTENT, 0600, NULL))
        g_warning("gnoblin: could not record the compositor crash");
}

/* The runtime directory can outlive a session, for example with lingering or a
 * second login, so a crash notice from an earlier session may still be there.
 * Drop it at startup; this session reports its own previous crash record. */
void discard_stale_crash_notice(void) {
    g_autofree char* path = config_fallback_marker_path();
    g_autofree char* existing = NULL;
    if (g_file_get_contents(path, &existing, NULL, NULL) && g_str_has_prefix(existing, "crash\n"))
        (void)g_unlink(path);
}

/* Publish the previous session's crash once, as a recovery-panel notice. Run
 * it after startup has committed the configuration, because a clean config
 * load clears the notice file. */
void report_previous_crash(void) {
    g_autofree char* path = crash_record_path();
    g_autofree char* contents = NULL;
    if (!g_file_get_contents(path, &contents, NULL, NULL))
        return;
    (void)g_unlink(path);

    gint64 signal_number = 0;
    gint64 exit_status = 0;
    gint64 uptime = 0;
    g_auto(GStrv) lines = g_strsplit(contents, "\n", -1);
    for (guint i = 0; lines[i]; i++) {
        if (g_str_has_prefix(lines[i], "signal="))
            signal_number = g_ascii_strtoll(lines[i] + 7, NULL, 10);
        else if (g_str_has_prefix(lines[i], "status="))
            exit_status = g_ascii_strtoll(lines[i] + 7, NULL, 10);
        else if (g_str_has_prefix(lines[i], "uptime="))
            uptime = g_ascii_strtoll(lines[i] + 7, NULL, 10);
    }
    g_autofree char* log =
        g_build_filename(g_get_user_state_dir(), "gnoblin", "compositor-previous.log", NULL);
    g_autofree char* what =
        signal_number ? g_strdup_printf("was stopped by signal %d (%s)", (int)signal_number,
                                        g_strsignal((int)signal_number))
                      : g_strdup_printf("exited with status %d", (int)exit_status);
    g_autofree char* message =
        g_strdup_printf("Gnoblin ended unexpectedly in the previous session. The compositor %s "
                        "after %" G_GINT64_FORMAT " seconds. Its log is at %s.",
                        what, uptime, log);
    g_autofree char* existing = config_recovery_notice();
    if (existing)
        append_config_notice(message);
    else
        set_config_notice("crash", message);
}

void set_reload_failure_notice(Runtime* runtime, const char* diagnostic) {
    const char* state = "notice";
    if (runtime && runtime->config_fallback)
        state = runtime->config_fallback_last_good ? "last-good" : "defaults";
    set_config_notice(state, diagnostic);
}
