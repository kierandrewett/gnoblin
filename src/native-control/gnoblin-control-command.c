/* Command spawning: configured shortcut commands, command.run and touchpad gesture commands.
 * A single worker thread runs the fork/exec handshake so that it cannot stall the compositor
 * main loop, and the main loop watches each child until it exits. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#include <dirent.h>
#include <errno.h>
#include <signal.h>
#include <spawn.h>
#include <string.h>
#include <unistd.h>

#define MAX_PENDING_NATIVE_COMMANDS 64

typedef struct {
    char* name;
    char* category;
    GPid pid;
} RunningCommand;

typedef struct {
    char* category;
    char* name;
    char** argv;
    gint64 queued_at_us;
} NativeCommandJob;

typedef struct {
    char* category;
    char* name;
    GPid pid;
    char* error_message;
    gint64 queued_at_us;
    gint64 spawn_started_us;
    gint64 spawn_finished_us;
} NativeCommandStart;

/* A single worker keeps configured command launches ordered without allowing
 * the synchronous fork/exec handshake to stall the compositor's input loop. */
static GThreadPool* native_command_executor;
static GOnce native_command_executor_once = G_ONCE_INIT;
static gint native_command_pending_count;

static void native_command_start_free(NativeCommandStart* start) {
    if (!start)
        return;
    g_free(start->category);
    g_free(start->name);
    g_free(start->error_message);
    g_free(start);
}

static void native_command_job_free(NativeCommandJob* job) {
    if (!job)
        return;
    g_free(job->category);
    g_free(job->name);
    g_strfreev(job->argv);
    g_free(job);
}

static void native_command_finished(GPid pid, gint status, gpointer user_data) {
    RunningCommand* run = user_data;
    if (!g_spawn_check_wait_status(status, NULL))
        g_warning("gnoblin-%s: %s exited unsuccessfully", run->category, run->name);
    g_spawn_close_pid(pid);
    g_free(run->name);
    g_free(run->category);
    g_free(run);
}

static gboolean native_command_started(gpointer user_data) {
    NativeCommandStart* start = user_data;
    g_atomic_int_add(&native_command_pending_count, -1);
    if (!start->pid) {
        g_warning("gnoblin-%s: could not start %s: %s", start->category, start->name,
                  start->error_message);
        return G_SOURCE_REMOVE;
    }

    RunningCommand* run = g_new0(RunningCommand, 1);
    run->name = g_strdup(start->name);
    run->category = g_strdup(start->category);
    run->pid = start->pid;
    start->pid = 0;
    g_child_watch_add(run->pid, native_command_finished, run);
    g_message("gnoblin-%s: started %s (queue %.1f ms, spawn %.1f ms)", start->category,
              start->name, (start->spawn_started_us - start->queued_at_us) / 1000.0,
              (start->spawn_finished_us - start->spawn_started_us) / 1000.0);
    return G_SOURCE_REMOVE;
}

void gnoblin_control_command_reap(GPid pid, gint status, gpointer user_data) {
    (void)status;
    (void)user_data;
    g_spawn_close_pid(pid);
}

static gboolean native_command_close_inherited_fds(posix_spawn_file_actions_t* actions,
                                                   GError** error) {
#ifdef __GLIBC__
    int result = posix_spawn_file_actions_addclosefrom_np(actions, STDERR_FILENO + 1);
    if (result == 0)
        return TRUE;
#endif
    /* Older libc has no closefrom spawn action. Build close actions from the
     * current descriptor table: this remains independent of compositor heap
     * size and does not mutate the parent's descriptor flags. */
    DIR* directory = opendir("/proc/self/fd");
    if (!directory) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "cannot enumerate inherited descriptors: %s", g_strerror(errno));
        return FALSE;
    }
    int directory_fd = dirfd(directory);
    struct dirent* entry;
    while ((entry = readdir(directory))) {
        char* end = NULL;
        errno = 0;
        long fd = strtol(entry->d_name, &end, 10);
        if (errno || !end || *end || fd <= STDERR_FILENO || fd == directory_fd)
            continue;
        int result = posix_spawn_file_actions_addclose(actions, (int)fd);
        if (result != 0) {
            closedir(directory);
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(result),
                        "cannot close inherited descriptor: %s", g_strerror(result));
            return FALSE;
        }
    }
    closedir(directory);
    return TRUE;
}

gboolean gnoblin_control_command_spawn(char** argv, GPid* pid, GError** error) {
    posix_spawn_file_actions_t actions;
    posix_spawnattr_t attributes;
    sigset_t empty_mask, default_signals;
    short flags = POSIX_SPAWN_SETSIGMASK | POSIX_SPAWN_SETSIGDEF;
    int result = posix_spawn_file_actions_init(&actions);
    if (result != 0)
        goto fail_actions;
    if (!native_command_close_inherited_fds(&actions, error)) {
        posix_spawn_file_actions_destroy(&actions);
        return FALSE;
    }
    result = posix_spawnattr_init(&attributes);
    if (result != 0)
        goto fail;
    sigemptyset(&empty_mask);
    sigfillset(&default_signals);
    result = posix_spawnattr_setsigmask(&attributes, &empty_mask);
    if (result == 0)
        result = posix_spawnattr_setsigdefault(&attributes, &default_signals);
#ifdef POSIX_SPAWN_SETSID
    flags |= POSIX_SPAWN_SETSID;
#endif
    if (result == 0)
        result = posix_spawnattr_setflags(&attributes, flags);
    if (result == 0)
        result = posix_spawnp(pid, argv[0], &actions, &attributes, argv, environ);
    posix_spawnattr_destroy(&attributes);
    posix_spawn_file_actions_destroy(&actions);
    if (result == 0)
        return TRUE;
fail_actions:
    g_set_error(error, G_IO_ERROR, g_io_error_from_errno(result), "could not spawn %s: %s",
                argv[0], g_strerror(result));
    return FALSE;
fail:
    posix_spawn_file_actions_destroy(&actions);
    goto fail_actions;
}

static void native_command_spawn_worker(gpointer data, gpointer user_data) {
    NativeCommandJob* job = data;
    NativeCommandStart* start = g_new0(NativeCommandStart, 1);
    (void)user_data;
    start->category = g_steal_pointer(&job->category);
    start->name = g_steal_pointer(&job->name);
    start->queued_at_us = job->queued_at_us;
    start->spawn_started_us = g_get_monotonic_time();
    g_autoptr(GError) error = NULL;
    if (!gnoblin_control_command_spawn(job->argv, &start->pid, &error))
        start->error_message = g_strdup(error->message);
    start->spawn_finished_us = g_get_monotonic_time();
    native_command_job_free(job);
    GSource* source = g_idle_source_new();
    g_source_set_priority(source, G_PRIORITY_DEFAULT);
    g_source_set_callback(source, native_command_started, start,
                          (GDestroyNotify)native_command_start_free);
    g_source_attach(source, g_main_context_default());
    g_source_unref(source);
}

static gpointer native_command_executor_init(gpointer unused) {
    GError* error = NULL;
    (void)unused;
    native_command_executor = g_thread_pool_new(native_command_spawn_worker, NULL, 1, FALSE, &error);
    if (!native_command_executor) {
        g_warning("gnoblin-shortcut: could not create command executor: %s", error->message);
        g_clear_error(&error);
    }
    return native_command_executor;
}

void gnoblin_control_launch_command(const char* category, const char* name, char** argv) {
    g_autoptr(GError) error = NULL;
    if (!gnoblin_control_queue_command(category, name, argv, &error))
        g_warning("gnoblin-%s: could not queue %s: %s", category, name,
                  error ? error->message : "unknown error");
}

/* Queueing is the only synchronous command outcome.  Spawn and child exit run
 * on the existing command worker and are intentionally reported to the log. */
gboolean gnoblin_control_queue_command(const char* category, const char* name, char** argv,
                                     GError** error) {
    if (!category || !*category || !name || !*name || !argv || !argv[0] || !*argv[0]) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "command needs a category, name, and nonempty argv[0]");
        return FALSE;
    }
    if (g_atomic_int_add(&native_command_pending_count, 1) >= MAX_PENDING_NATIVE_COMMANDS) {
        g_atomic_int_add(&native_command_pending_count, -1);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                            "command queue is full");
        return FALSE;
    }
    GThreadPool* executor = g_once(&native_command_executor_once, native_command_executor_init, NULL);
    if (!executor) {
        g_atomic_int_add(&native_command_pending_count, -1);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            "command executor is unavailable");
        return FALSE;
    }
    NativeCommandJob* job = g_new0(NativeCommandJob, 1);
    job->category = g_strdup(category);
    job->name = g_strdup(name);
    job->argv = g_strdupv(argv);
    job->queued_at_us = g_get_monotonic_time();
    if (!g_thread_pool_push(executor, job, error)) {
        g_atomic_int_add(&native_command_pending_count, -1);
        native_command_job_free(job);
        return FALSE;
    }
    return TRUE;
}

char** gnoblin_control_command_run_argv(GVariant* arguments, GError** error) {
    g_autoptr(GVariant) argv_value =
        arguments ? g_variant_lookup_value(arguments, "argv", G_VARIANT_TYPE_STRING_ARRAY) : NULL;
    if (!arguments || g_variant_n_children(arguments) != 1 || !argv_value ||
        g_variant_n_children(argv_value) == 0 ||
        g_variant_n_children(argv_value) > MAX_NATIVE_COMMAND_ARGUMENTS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "command.run requires an argv array of 1 to 64 strings");
        return NULL;
    }

    char** argv = g_new0(char*, g_variant_n_children(argv_value) + 1);
    gsize bytes = 0;
    for (gsize index = 0; index < g_variant_n_children(argv_value); index++) {
        g_autoptr(GVariant) argument = g_variant_get_child_value(argv_value, index);
        gsize length = 0;
        const char* text = g_variant_get_string(argument, &length);
        if ((index == 0 && length == 0) || memchr(text, '\0', length) ||
            !g_utf8_validate(text, length, NULL) || length > MAX_NATIVE_COMMAND_BYTES - 1 ||
            bytes > MAX_NATIVE_COMMAND_BYTES - length - 1) {
            g_strfreev(argv);
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "command.run argv is invalid or exceeds 64 KiB");
            return NULL;
        }
        argv[index] = g_strndup(text, length);
        bytes += length + 1;
    }
    return argv;
}
