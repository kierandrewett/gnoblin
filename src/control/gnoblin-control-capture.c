/* command.capture: runs a command for Lua and returns its standard output. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#include <signal.h>
#include <string.h>

/* command.capture: run a command, collect its standard output, and complete the Lua operation
 * when it exits. The command inherits no stdin, and its stderr is discarded. The output is
 * bounded and the run is time limited, because the caller may be answering a password
 * prompt and must never wait on a stuck program. */
#define MAX_PENDING_CAPTURES 4
#define CAPTURE_DEFAULT_TIMEOUT_SECONDS 60
#define CAPTURE_MAX_TIMEOUT_SECONDS 600
#define CAPTURE_MAX_OUTPUT_BYTES (64 * 1024)
#define CAPTURE_MAX_STDIN_BYTES 4096

typedef struct {
    gatomicrefcount refs;
    GnoblinNativeControl* control;
    gint64 operation_id;
    char** argv;
    char* stdin_text;
    guint timeout_seconds;
    GSubprocess* subprocess;
    GCancellable* cancellable;
    GByteArray* output;
    guint timeout_source_id;
    gboolean timed_out;
    gboolean truncated;
    gboolean stdout_done;
    gboolean exit_done;
    gboolean finished;
} PendingCapture;

static PendingCapture* capture_ref(PendingCapture* capture) {
    g_atomic_ref_count_inc(&capture->refs);
    return capture;
}

static void capture_unref(gpointer data) {
    PendingCapture* capture = data;
    if (!g_atomic_ref_count_dec(&capture->refs))
        return;
    if (capture->timeout_source_id)
        g_source_remove(capture->timeout_source_id);
    g_strfreev(capture->argv);
    /* The input and the output can hold a password. Wipe them before they are freed. */
    if (capture->stdin_text) {
        memset(capture->stdin_text, 0, strlen(capture->stdin_text));
        g_free(capture->stdin_text);
    }
    if (capture->output) {
        if (capture->output->len)
            memset(capture->output->data, 0, capture->output->len);
        g_byte_array_free(capture->output, TRUE);
    }
    g_clear_object(&capture->subprocess);
    g_clear_object(&capture->cancellable);
    g_free(capture);
}

static void capture_complete(PendingCapture* capture, JsonNode* result, const GError* error) {
    GnoblinNativeControl* control = capture->control;
    capture->finished = TRUE;
    if (!control || control->stopping)
        return;
    gnoblin_control_dispatch_operation_completion_full(
        control, capture->operation_id, "command.capture", result != NULL, result,
        gnoblin_control_operation_error_code(error), error ? error->message : NULL, FALSE);
}

static void capture_remove(PendingCapture* capture) {
    GnoblinNativeControl* control = capture->control;
    if (control && control->pending_captures)
        g_hash_table_remove(control->pending_captures, capture);
}

static void capture_finish(PendingCapture* capture) {
    if (capture->finished || !capture->stdout_done || !capture->exit_done)
        return;
    capture_ref(capture);

    g_autoptr(GError) error = NULL;
    g_autoptr(JsonNode) result = NULL;
    if (capture->timed_out) {
        g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_TIMED_OUT, "command timed out");
    } else if (capture->truncated) {
        g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                            "command output exceeded 64 KiB");
    } else if (!g_utf8_validate((const char*)capture->output->data, capture->output->len, NULL)) {
        g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "command output is not valid UTF-8");
    } else {
        int exit_code = g_subprocess_get_if_exited(capture->subprocess)
                            ? g_subprocess_get_exit_status(capture->subprocess)
                            : -g_subprocess_get_term_sig(capture->subprocess);
        /* An empty GByteArray has no data pointer, and g_strndup() returns NULL for it. */
        g_autofree char* text = g_strndup(
            capture->output->len ? (const char*)capture->output->data : "", capture->output->len);
        JsonObject* object = json_object_new();
        json_object_set_int_member(object, "exit_code", exit_code);
        json_object_set_string_member(object, "stdout", text);
        result = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(result, object);
        capture_complete(capture, result, NULL);
        memset(text, 0, strlen(text));
    }
    if (error)
        capture_complete(capture, NULL, error);

    capture_remove(capture);
    capture_unref(capture);
}

static void capture_stop_io(PendingCapture* capture) {
    if (capture->subprocess)
        g_subprocess_force_exit(capture->subprocess);
    if (capture->cancellable)
        g_cancellable_cancel(capture->cancellable);
}

static void capture_read_more(PendingCapture* capture);

static void capture_read_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    PendingCapture* capture = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GBytes) chunk =
        g_input_stream_read_bytes_finish(G_INPUT_STREAM(source), result, &error);
    gsize length = chunk ? g_bytes_get_size(chunk) : 0;

    if (length > 0 && !capture->finished && !capture->stdout_done) {
        if (capture->output->len + length > CAPTURE_MAX_OUTPUT_BYTES) {
            capture->truncated = TRUE;
            capture->stdout_done = TRUE;
            capture_stop_io(capture);
        } else {
            g_byte_array_append(capture->output, g_bytes_get_data(chunk, NULL), length);
            capture_read_more(capture);
            capture_unref(capture);
            return;
        }
    } else {
        capture->stdout_done = TRUE;
    }
    capture_finish(capture);
    capture_unref(capture);
}

static void capture_read_more(PendingCapture* capture) {
    g_input_stream_read_bytes_async(g_subprocess_get_stdout_pipe(capture->subprocess), 4096,
                                    G_PRIORITY_DEFAULT, capture->cancellable, capture_read_done,
                                    capture_ref(capture));
}

static void capture_wait_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    PendingCapture* capture = user_data;
    g_autoptr(GError) error = NULL;
    g_subprocess_wait_finish(G_SUBPROCESS(source), result, &error);
    capture->exit_done = TRUE;
    capture_finish(capture);
    capture_unref(capture);
}

static gboolean capture_timeout(gpointer user_data) {
    PendingCapture* capture = user_data;
    capture->timeout_source_id = 0;
    capture->timed_out = TRUE;
    capture_stop_io(capture);
    return G_SOURCE_REMOVE;
}

static void capture_stdin_closed(GObject* source, GAsyncResult* result, gpointer user_data) {
    g_autoptr(GError) error = NULL;
    g_output_stream_close_finish(G_OUTPUT_STREAM(source), result, &error);
    capture_unref(user_data);
}

static void capture_stdin_written(GObject* source, GAsyncResult* result, gpointer user_data) {
    PendingCapture* capture = user_data;
    g_autoptr(GError) error = NULL;
    g_output_stream_write_all_finish(G_OUTPUT_STREAM(source), result, NULL, &error);
    g_output_stream_close_async(G_OUTPUT_STREAM(source), G_PRIORITY_DEFAULT, capture->cancellable,
                                capture_stdin_closed, capture);
}

static void capture_child_setup(gpointer user_data) {
    /* The compositor blocks signals on its threads. A child must start with none blocked. */
    sigset_t empty;
    sigemptyset(&empty);
    sigprocmask(SIG_SETMASK, &empty, NULL);
}

static void capture_spawn_thread(GTask* task, gpointer source, gpointer task_data,
                                 GCancellable* cancellable) {
    PendingCapture* capture = task_data;
    GError* error = NULL;
    g_autoptr(GSubprocessLauncher) launcher =
        g_subprocess_launcher_new(G_SUBPROCESS_FLAGS_STDIN_PIPE | G_SUBPROCESS_FLAGS_STDOUT_PIPE |
                                  G_SUBPROCESS_FLAGS_STDERR_SILENCE);
    g_subprocess_launcher_set_child_setup(launcher, capture_child_setup, NULL, NULL);
    GSubprocess* subprocess =
        g_subprocess_launcher_spawnv(launcher, (const char* const*)capture->argv, &error);
    if (subprocess)
        g_task_return_pointer(task, subprocess, g_object_unref);
    else
        g_task_return_error(task, error);
}

static void capture_spawned(GObject* source, GAsyncResult* result, gpointer user_data) {
    PendingCapture* capture = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GSubprocess) subprocess = g_task_propagate_pointer(G_TASK(result), &error);

    if (!subprocess || !capture->control || capture->control->stopping) {
        if (subprocess)
            g_subprocess_force_exit(subprocess);
        if (!subprocess) {
            capture_complete(capture, NULL, error);
            capture_remove(capture);
        }
        capture_unref(capture);
        return;
    }

    capture->subprocess = g_steal_pointer(&subprocess);
    capture->cancellable = g_cancellable_new();
    capture->output = g_byte_array_new();

    GOutputStream* input = g_subprocess_get_stdin_pipe(capture->subprocess);
    if (capture->stdin_text && *capture->stdin_text)
        g_output_stream_write_all_async(input, capture->stdin_text, strlen(capture->stdin_text),
                                        G_PRIORITY_DEFAULT, capture->cancellable,
                                        capture_stdin_written, capture_ref(capture));
    else
        g_output_stream_close_async(input, G_PRIORITY_DEFAULT, capture->cancellable,
                                    capture_stdin_closed, capture_ref(capture));
    capture_read_more(capture);
    g_subprocess_wait_async(capture->subprocess, capture->cancellable, capture_wait_done,
                            capture_ref(capture));
    capture->timeout_source_id =
        g_timeout_add_seconds(capture->timeout_seconds, capture_timeout, capture);
    capture_unref(capture);
}

void gnoblin_control_capture_clear(GnoblinNativeControl* control) {
    if (!control || !control->pending_captures)
        return;
    GHashTableIter iter;
    gpointer key;
    g_hash_table_iter_init(&iter, control->pending_captures);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        PendingCapture* capture = key;
        capture->control = NULL;
        capture_stop_io(capture);
    }
    g_hash_table_remove_all(control->pending_captures);
}

gboolean gnoblin_control_capture_begin(GnoblinNativeControl* control, gint64 operation_id,
                                       GVariant* arguments, GError** error) {
    g_autoptr(GVariant) argv_value =
        g_variant_lookup_value(arguments, "argv", G_VARIANT_TYPE_STRING_ARRAY);
    gsize count = argv_value ? g_variant_n_children(argv_value) : 0;
    if (count < 1 || count > MAX_NATIVE_COMMAND_ARGUMENTS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "command.capture requires an argv array of 1 to 64 strings");
        return FALSE;
    }
    g_auto(GStrv) argv = g_variant_dup_strv(argv_value, NULL);
    gsize bytes = 0;
    for (gsize i = 0; argv[i]; i++)
        bytes += strlen(argv[i]) + 1;
    if (!*argv[0] || bytes > MAX_NATIVE_COMMAND_BYTES) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "command.capture argv is empty or exceeds 64 KiB");
        return FALSE;
    }

    const char* stdin_text = NULL;
    gint64 timeout = CAPTURE_DEFAULT_TIMEOUT_SECONDS;
    g_variant_lookup(arguments, "stdin", "&s", &stdin_text);
    g_variant_lookup(arguments, "timeout", "x", &timeout);
    if ((stdin_text && strlen(stdin_text) > CAPTURE_MAX_STDIN_BYTES) || timeout < 1 ||
        timeout > CAPTURE_MAX_TIMEOUT_SECONDS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "command.capture stdin is limited to 4096 bytes and timeout to 1-600");
        return FALSE;
    }
    if (!control->pending_captures ||
        g_hash_table_size(control->pending_captures) >= MAX_PENDING_CAPTURES) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                            "too many commands are already running");
        return FALSE;
    }

    PendingCapture* capture = g_new0(PendingCapture, 1);
    g_atomic_ref_count_init(&capture->refs);
    capture->control = control;
    capture->operation_id = operation_id;
    capture->argv = g_steal_pointer(&argv);
    capture->stdin_text = stdin_text ? g_strdup(stdin_text) : NULL;
    capture->timeout_seconds = (guint)timeout;
    g_hash_table_add(control->pending_captures, capture_ref(capture));

    /* Forking a large process can stall the main loop, so spawn on a worker thread. */
    g_autoptr(GTask) task = g_task_new(NULL, NULL, capture_spawned, capture_ref(capture));
    g_task_set_task_data(task, capture, NULL);
    g_task_run_in_thread(task, capture_spawn_thread);
    return TRUE;
}
void gnoblin_control_capture_init(GnoblinNativeControl* control) {
    control->pending_captures =
        g_hash_table_new_full(g_direct_hash, g_direct_equal, capture_unref, NULL);
}
