/* process.spawn and process.kill: processes the Lua runtime starts and controls.
 *
 * The compositor spawns, because it owns the session environment and must know each child to
 * attribute its windows. Each child is a session leader (setsid), so a signal reaches its whole
 * process group. process.spawn completes with the pid once the child exists; its exit arrives
 * later as the runtime event gnoblin.process.exited, so a long-lived child never holds a config
 * reload behind a pending operation. Captured output and stdin can hold secrets and are wiped
 * before they are freed. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"
#include "core/gnoblin-native-control-private.h"

#include <signal.h>
#include <string.h>
#include <unistd.h>

#define MAX_RUNNING_PROCESSES 64
#define MAX_PROCESS_STDIN_BYTES (1024 * 1024)
#define DEFAULT_PROCESS_OUTPUT_LIMIT (64 * 1024)
#define MAX_PROCESS_OUTPUT_LIMIT (4 * 1024 * 1024)
#define MAX_PROCESS_TIMEOUT_MS (86400 * 1000)
#define MAX_PROCESS_ENV_ENTRIES 128
#define PROCESS_KILL_GRACE_MS 2000
#define PROCESS_DRAIN_GRACE_MS 2000

typedef enum { STREAM_INHERIT, STREAM_DISCARD, STREAM_CAPTURE } StreamMode;

typedef struct {
    GInputStream* stream;
    GByteArray* data;
    gboolean done;
    gboolean truncated;
} CapturedStream;

typedef struct {
    gatomicrefcount refs;
    GnoblinNativeControl* control;
    gint64 id; /* The process.spawn request ID; Lua's Process.id. */
    char** argv;
    char** env;
    char* cwd;
    GBytes* stdin_bytes;
    StreamMode stdout_mode, stderr_mode;
    gsize output_limit;
    guint timeout_ms;
    GSubprocess* subprocess;
    pid_t pid;
    GCancellable* cancellable;
    CapturedStream out, err;
    guint timeout_source_id, kill_source_id, drain_source_id;
    gboolean timed_out;
    gboolean exited;
    gboolean reported;
} RunningProcess;

static RunningProcess* process_ref(RunningProcess* process) {
    g_atomic_ref_count_inc(&process->refs);
    return process;
}

static void wipe_bytes(GByteArray* bytes) {
    if (!bytes)
        return;
    if (bytes->len)
        memset(bytes->data, 0, bytes->len);
    g_byte_array_free(bytes, TRUE);
}

static void process_unref(gpointer data) {
    RunningProcess* process = data;
    if (!g_atomic_ref_count_dec(&process->refs))
        return;
    g_clear_handle_id(&process->timeout_source_id, g_source_remove);
    g_clear_handle_id(&process->kill_source_id, g_source_remove);
    g_clear_handle_id(&process->drain_source_id, g_source_remove);
    g_strfreev(process->argv);
    g_strfreev(process->env);
    g_free(process->cwd);
    if (process->stdin_bytes) {
        gsize size = 0;
        gpointer data_copy = g_bytes_unref_to_data(process->stdin_bytes, &size);
        if (size)
            memset(data_copy, 0, size);
        g_free(data_copy);
    }
    wipe_bytes(process->out.data);
    wipe_bytes(process->err.data);
    g_clear_object(&process->subprocess);
    g_clear_object(&process->cancellable);
    g_free(process);
}

static void process_signal_group(RunningProcess* process, int signal_number) {
    if (process->pid > 0 && !process->exited)
        kill(-process->pid, signal_number);
}

static GVariant* captured_variant(CapturedStream* stream) {
    const void* bytes = stream->data->len ? (const void*)stream->data->data : "";
    return g_variant_new_fixed_array(G_VARIANT_TYPE_BYTE, bytes, stream->data->len, 1);
}

/* Report the exit once the child is collected and every captured stream reached its end. */
static void process_report(RunningProcess* process) {
    if (process->reported || !process->exited || (process->out.data && !process->out.done) ||
        (process->err.data && !process->err.done))
        return;
    process->reported = TRUE;
    GnoblinNativeControl* control = process->control;
    if (control && control->processes)
        g_hash_table_remove(control->processes, &process->id);
    if (!control || control->stopping)
        return;

    GVariantBuilder exit;
    g_variant_builder_init(&exit, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&exit, "{sv}", "id", g_variant_new_int64(process->id));
    if (g_subprocess_get_if_exited(process->subprocess))
        g_variant_builder_add(
            &exit, "{sv}", "exit_code",
            g_variant_new_int64(g_subprocess_get_exit_status(process->subprocess)));
    else
        g_variant_builder_add(&exit, "{sv}", "signal",
                              g_variant_new_int64(g_subprocess_get_term_sig(process->subprocess)));
    g_variant_builder_add(&exit, "{sv}", "timed_out", g_variant_new_boolean(process->timed_out));
    if (process->out.data) {
        g_variant_builder_add(&exit, "{sv}", "stdout", captured_variant(&process->out));
        g_variant_builder_add(&exit, "{sv}", "stdout_truncated",
                              g_variant_new_boolean(process->out.truncated));
    }
    if (process->err.data) {
        g_variant_builder_add(&exit, "{sv}", "stderr", captured_variant(&process->err));
        g_variant_builder_add(&exit, "{sv}", "stderr_truncated",
                              g_variant_new_boolean(process->err.truncated));
    }
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&exit));
    native_runtime_dispatch_event(control, "gnoblin.process.exited", payload);
}

static void process_read_more(RunningProcess* process, CapturedStream* stream);

typedef struct {
    RunningProcess* process;
    CapturedStream* stream;
} StreamRead;

static void process_read_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    StreamRead* read = user_data;
    RunningProcess* process = read->process;
    CapturedStream* stream = read->stream;
    g_free(read);
    g_autoptr(GBytes) chunk =
        g_input_stream_read_bytes_finish(G_INPUT_STREAM(source), result, NULL);
    gsize length = chunk ? g_bytes_get_size(chunk) : 0;
    if (length == 0 || process->reported) {
        stream->done = TRUE;
        process_report(process);
        process_unref(process);
        return;
    }
    /* Past the limit, keep reading and drop the rest so the child never blocks on a full pipe. */
    gsize room = process->output_limit - MIN(stream->data->len, process->output_limit);
    if (length > room)
        stream->truncated = TRUE;
    g_byte_array_append(stream->data, g_bytes_get_data(chunk, NULL), MIN(length, room));
    process_read_more(process, stream);
    process_unref(process);
}

static void process_read_more(RunningProcess* process, CapturedStream* stream) {
    StreamRead* read = g_new(StreamRead, 1);
    read->process = process_ref(process);
    read->stream = stream;
    g_input_stream_read_bytes_async(stream->stream, 16384, G_PRIORITY_DEFAULT, process->cancellable,
                                    process_read_done, read);
}

/* A grandchild can keep a captured pipe open after the child exits. Stop waiting for it. */
static gboolean process_drain_expired(gpointer user_data) {
    RunningProcess* process = user_data;
    process->drain_source_id = 0;
    g_cancellable_cancel(process->cancellable);
    return G_SOURCE_REMOVE;
}

static void process_wait_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    RunningProcess* process = user_data;
    g_subprocess_wait_finish(G_SUBPROCESS(source), result, NULL);
    process->exited = TRUE;
    g_clear_handle_id(&process->timeout_source_id, g_source_remove);
    g_clear_handle_id(&process->kill_source_id, g_source_remove);
    if ((process->out.data && !process->out.done) || (process->err.data && !process->err.done))
        process->drain_source_id =
            g_timeout_add_full(G_PRIORITY_DEFAULT, PROCESS_DRAIN_GRACE_MS, process_drain_expired,
                               process_ref(process), process_unref);
    process_report(process);
    process_unref(process);
}

static gboolean process_force_kill(gpointer user_data) {
    RunningProcess* process = user_data;
    process->kill_source_id = 0;
    process_signal_group(process, SIGKILL);
    return G_SOURCE_REMOVE;
}

static void process_terminate(RunningProcess* process) {
    process_signal_group(process, SIGTERM);
    if (!process->kill_source_id && !process->exited)
        process->kill_source_id =
            g_timeout_add_full(G_PRIORITY_DEFAULT, PROCESS_KILL_GRACE_MS, process_force_kill,
                               process_ref(process), process_unref);
}

static gboolean process_timeout(gpointer user_data) {
    RunningProcess* process = user_data;
    process->timeout_source_id = 0;
    process->timed_out = TRUE;
    process_terminate(process);
    return G_SOURCE_REMOVE;
}

static void process_stdin_closed(GObject* source, GAsyncResult* result, gpointer user_data) {
    g_output_stream_close_finish(G_OUTPUT_STREAM(source), result, NULL);
    process_unref(user_data);
}

static void process_stdin_written(GObject* source, GAsyncResult* result, gpointer user_data) {
    RunningProcess* process = user_data;
    g_output_stream_write_all_finish(G_OUTPUT_STREAM(source), result, NULL, NULL);
    g_output_stream_close_async(G_OUTPUT_STREAM(source), G_PRIORITY_DEFAULT, process->cancellable,
                                process_stdin_closed, process);
}

static void process_child_setup(gpointer user_data) {
    /* The compositor blocks signals on its threads. A child must start with none blocked. */
    sigset_t empty;
    sigemptyset(&empty);
    sigprocmask(SIG_SETMASK, &empty, NULL);
    /* Its own session and process group, so process.kill reaches everything it starts. */
    setsid();
    /* Reset only the forked child, never the shared GTask worker. These calls do not allocate or
     * take GLib locks. Positive nice is left unchanged. */
    (void)gnoblin_control_reset_command_scheduling();
}

static GSubprocessFlags stream_flags(StreamMode mode, gboolean is_stdout) {
    if (mode == STREAM_CAPTURE)
        return is_stdout ? G_SUBPROCESS_FLAGS_STDOUT_PIPE : G_SUBPROCESS_FLAGS_STDERR_PIPE;
    if (mode == STREAM_DISCARD)
        return is_stdout ? G_SUBPROCESS_FLAGS_STDOUT_SILENCE : G_SUBPROCESS_FLAGS_STDERR_SILENCE;
    return G_SUBPROCESS_FLAGS_NONE;
}

static void process_spawn_thread(GTask* task, gpointer source, gpointer task_data,
                                 GCancellable* cancellable) {
    RunningProcess* process = task_data;
    GError* error = NULL;
    GSubprocessFlags flags =
        stream_flags(process->stdout_mode, TRUE) | stream_flags(process->stderr_mode, FALSE) |
        (process->stdin_bytes ? G_SUBPROCESS_FLAGS_STDIN_PIPE : G_SUBPROCESS_FLAGS_NONE);
    g_autoptr(GSubprocessLauncher) launcher = g_subprocess_launcher_new(flags);
    g_subprocess_launcher_set_child_setup(launcher, process_child_setup, NULL, NULL);
    if (process->cwd)
        g_subprocess_launcher_set_cwd(launcher, process->cwd);
    for (gsize i = 0; process->env && process->env[i]; i += 2)
        g_subprocess_launcher_setenv(launcher, process->env[i], process->env[i + 1], TRUE);
    GSubprocess* subprocess =
        g_subprocess_launcher_spawnv(launcher, (const char* const*)process->argv, &error);
    if (!subprocess) {
        /* Report a missing or forbidden program with the codes Lua callers check. */
        if (g_error_matches(error, G_SPAWN_ERROR, G_SPAWN_ERROR_NOENT)) {
            error->domain = G_IO_ERROR;
            error->code = G_IO_ERROR_NOT_FOUND;
        } else if (g_error_matches(error, G_SPAWN_ERROR, G_SPAWN_ERROR_ACCES) ||
                   g_error_matches(error, G_SPAWN_ERROR, G_SPAWN_ERROR_PERM)) {
            error->domain = G_IO_ERROR;
            error->code = G_IO_ERROR_PERMISSION_DENIED;
        }
        g_task_return_error(task, error);
        return;
    }
    /* Read the pid now: GSubprocess forgets it once a quick child has been reaped. */
    const char* identifier = g_subprocess_get_identifier(subprocess);
    process->pid = identifier ? (pid_t)g_ascii_strtoll(identifier, NULL, 10) : 0;
    g_task_return_pointer(task, subprocess, g_object_unref);
}

static void process_complete_spawn(RunningProcess* process, const GError* error) {
    GnoblinNativeControl* control = process->control;
    if (!control || control->stopping)
        return;
    g_autoptr(JsonNode) result = NULL;
    if (!error) {
        JsonObject* object = json_object_new();
        if (process->pid > 0)
            json_object_set_int_member(object, "pid", process->pid);
        result = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(result, object);
    }
    gnoblin_control_dispatch_operation_completion_full(
        control, process->id, "process.spawn", result != NULL, result,
        gnoblin_control_operation_error_code(error), error ? error->message : NULL, FALSE);
}

static void process_spawned(GObject* source, GAsyncResult* result, gpointer user_data) {
    RunningProcess* process = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GSubprocess) subprocess = g_task_propagate_pointer(G_TASK(result), &error);
    GnoblinNativeControl* control = process->control;
    if (!subprocess || !control || control->stopping) {
        if (subprocess && process->pid > 0)
            kill(-process->pid, SIGKILL);
        process_complete_spawn(process, error);
        if (control && control->processes)
            g_hash_table_remove(control->processes, &process->id);
        process_unref(process);
        return;
    }
    process->subprocess = g_steal_pointer(&subprocess);
    process->cancellable = g_cancellable_new();
    process_complete_spawn(process, NULL);

    if (process->stdin_bytes) {
        GOutputStream* input = g_subprocess_get_stdin_pipe(process->subprocess);
        gsize size = 0;
        const void* data = g_bytes_get_data(process->stdin_bytes, &size);
        if (size)
            g_output_stream_write_all_async(input, data, size, G_PRIORITY_DEFAULT,
                                            process->cancellable, process_stdin_written,
                                            process_ref(process));
        else
            g_output_stream_close_async(input, G_PRIORITY_DEFAULT, process->cancellable,
                                        process_stdin_closed, process_ref(process));
    }
    if (process->stdout_mode == STREAM_CAPTURE) {
        process->out.data = g_byte_array_new();
        process->out.stream = g_subprocess_get_stdout_pipe(process->subprocess);
        process_read_more(process, &process->out);
    }
    if (process->stderr_mode == STREAM_CAPTURE) {
        process->err.data = g_byte_array_new();
        process->err.stream = g_subprocess_get_stderr_pipe(process->subprocess);
        process_read_more(process, &process->err);
    }
    g_subprocess_wait_async(process->subprocess, NULL, process_wait_done, process_ref(process));
    if (process->timeout_ms)
        process->timeout_source_id =
            g_timeout_add_full(G_PRIORITY_DEFAULT, process->timeout_ms, process_timeout,
                               process_ref(process), process_unref);
    process_unref(process);
}

static gboolean stream_mode(GVariant* arguments, const char* key, StreamMode* mode,
                            GError** error) {
    const char* value = "inherit";
    g_variant_lookup(arguments, key, "&s", &value);
    if (g_str_equal(value, "inherit"))
        *mode = STREAM_INHERIT;
    else if (g_str_equal(value, "discard"))
        *mode = STREAM_DISCARD;
    else if (g_str_equal(value, "capture"))
        *mode = STREAM_CAPTURE;
    else {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "process.spawn %s must be inherit, discard or capture", key);
        return FALSE;
    }
    return TRUE;
}

gboolean gnoblin_control_process_spawn(GnoblinNativeControl* control, gint64 id,
                                       GVariant* arguments, GError** error) {
    g_autoptr(GVariant) argv_value =
        g_variant_lookup_value(arguments, "argv", G_VARIANT_TYPE_STRING_ARRAY);
    gsize count = argv_value ? g_variant_n_children(argv_value) : 0;
    if (count < 1 || count > MAX_NATIVE_COMMAND_ARGUMENTS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "process.spawn requires an argv array of 1 to 64 strings");
        return FALSE;
    }
    g_auto(GStrv) argv = g_variant_dup_strv(argv_value, NULL);
    gsize bytes = 0;
    for (gsize i = 0; argv[i]; i++)
        bytes += strlen(argv[i]) + 1;
    if (!*argv[0] || bytes > MAX_NATIVE_COMMAND_BYTES) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "process.spawn argv is empty or exceeds 64 KiB");
        return FALSE;
    }
    StreamMode stdout_mode, stderr_mode;
    if (!stream_mode(arguments, "stdout", &stdout_mode, error) ||
        !stream_mode(arguments, "stderr", &stderr_mode, error))
        return FALSE;
    gint64 output_limit = DEFAULT_PROCESS_OUTPUT_LIMIT;
    gint64 timeout_ms = 0;
    g_variant_lookup(arguments, "output_limit", "x", &output_limit);
    g_variant_lookup(arguments, "timeout_ms", "x", &timeout_ms);
    if (output_limit < 0 || output_limit > MAX_PROCESS_OUTPUT_LIMIT || timeout_ms < 0 ||
        timeout_ms > MAX_PROCESS_TIMEOUT_MS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "process.spawn output_limit or timeout is out of range");
        return FALSE;
    }
    const char* cwd = NULL;
    g_variant_lookup(arguments, "cwd", "&s", &cwd);
    if (cwd && !g_path_is_absolute(cwd)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "process.spawn cwd must be an absolute path");
        return FALSE;
    }
    g_autoptr(GVariant) stdin_value =
        g_variant_lookup_value(arguments, "stdin", G_VARIANT_TYPE_BYTESTRING);
    if (stdin_value && g_variant_get_size(stdin_value) > MAX_PROCESS_STDIN_BYTES) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "process.spawn stdin exceeds 1 MiB");
        return FALSE;
    }
    g_autoptr(GVariant) env_value =
        g_variant_lookup_value(arguments, "env", G_VARIANT_TYPE("a{ss}"));
    g_autoptr(GPtrArray) env = g_ptr_array_new_with_free_func(g_free);
    if (env_value) {
        if (g_variant_n_children(env_value) > MAX_PROCESS_ENV_ENTRIES) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "process.spawn env has more than 128 entries");
            return FALSE;
        }
        GVariantIter iter;
        const char *name, *value;
        g_variant_iter_init(&iter, env_value);
        while (g_variant_iter_next(&iter, "{&s&s}", &name, &value)) {
            if (!*name || strchr(name, '=')) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                    "process.spawn env names must be non-empty and contain no '='");
                return FALSE;
            }
            g_ptr_array_add(env, g_strdup(name));
            g_ptr_array_add(env, g_strdup(value));
        }
    }
    g_ptr_array_add(env, NULL);
    if (!control->processes || g_hash_table_size(control->processes) >= MAX_RUNNING_PROCESSES) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "too many processes are already running");
        return FALSE;
    }

    RunningProcess* process = g_new0(RunningProcess, 1);
    g_atomic_ref_count_init(&process->refs);
    process->control = control;
    process->id = id;
    process->argv = g_steal_pointer(&argv);
    process->env = (char**)g_ptr_array_free(g_steal_pointer(&env), FALSE);
    process->cwd = g_strdup(cwd);
    if (stdin_value)
        process->stdin_bytes =
            g_bytes_new(g_variant_get_data(stdin_value), g_variant_get_size(stdin_value));
    process->stdout_mode = stdout_mode;
    process->stderr_mode = stderr_mode;
    process->output_limit = (gsize)output_limit;
    process->timeout_ms = (guint)timeout_ms;
    g_hash_table_insert(control->processes, &process->id, process_ref(process));

    /* Forking a large process can stall the main loop, so spawn on a worker thread. */
    g_autoptr(GTask) task = g_task_new(NULL, NULL, process_spawned, process);
    g_task_set_task_data(task, process, NULL);
    g_task_run_in_thread(task, process_spawn_thread);
    return TRUE;
}

GVariant* gnoblin_control_process_kill(GnoblinNativeControl* control, GVariant* arguments,
                                       GError** error) {
    static const struct {
        const char* name;
        int number;
    } signals[] = {
        {"term", SIGTERM}, {"kill", SIGKILL}, {"int", SIGINT},   {"hup", SIGHUP},
        {"usr1", SIGUSR1}, {"usr2", SIGUSR2}, {"stop", SIGSTOP}, {"cont", SIGCONT},
    };
    gint64 id = 0;
    const char* name = "term";
    g_variant_lookup(arguments, "signal", "&s", &name);
    int number = 0;
    for (gsize i = 0; i < G_N_ELEMENTS(signals); i++)
        if (g_str_equal(name, signals[i].name))
            number = signals[i].number;
    if (!g_variant_lookup(arguments, "id", "x", &id) || id <= 0 || !number) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "process.kill requires a process id and a known signal name");
        return NULL;
    }
    RunningProcess* process =
        control->processes ? g_hash_table_lookup(control->processes, &id) : NULL;
    if (!process || process->pid <= 0 || process->exited) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "the process is not running");
        return NULL;
    }
    process_signal_group(process, number);
    GVariantBuilder sent;
    g_variant_builder_init(&sent, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&sent, "{sv}", "signal", g_variant_new_string(name));
    return g_variant_ref_sink(g_variant_builder_end(&sent));
}

/* The process ID the runtime used to spawn `pid`, or 0. A window belongs to a spawned process
 * when its client is that process or a member of its session. */
gint64 gnoblin_control_process_for_pid(GnoblinNativeControl* control, pid_t pid) {
    if (!control || !control->processes || pid <= 0)
        return 0;
    pid_t session = getsid(pid);
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->processes);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        RunningProcess* process = value;
        if (process->pid > 0 && !process->exited &&
            (process->pid == pid || process->pid == session))
            return process->id;
    }
    return 0;
}

void gnoblin_control_process_init(GnoblinNativeControl* control) {
    control->processes = g_hash_table_new_full(g_int64_hash, g_int64_equal, NULL, process_unref);
}

/* Compositor shutdown: send SIGTERM to each process group the runtime started and stop tracking
 * it. A child that ignores SIGTERM keeps running; the session's own teardown ends it. */
void gnoblin_control_process_clear(GnoblinNativeControl* control) {
    if (!control || !control->processes)
        return;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->processes);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        RunningProcess* process = value;
        process->control = NULL;
        process_signal_group(process, SIGTERM);
        if (process->cancellable)
            g_cancellable_cancel(process->cancellable);
    }
    g_hash_table_remove_all(control->processes);
}
