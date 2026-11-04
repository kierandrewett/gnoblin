#!/usr/bin/env python3
"""Link the real command executors without a compositor or display.

Only the control-plane completion boundary is stubbed. Scheduling, spawning,
GLib workers, subprocess pipes and child watches use their real implementations.
Run the compiled probe at negative nice to exercise boosted-parent inheritance.
"""

import argparse
import hashlib
from pathlib import Path
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCES = [ROOT / "src/control" / name for name in ("gnoblin-control-command.c", "gnoblin-control-process.c")]

HEADER = r"""
#pragma once
#include <gio/gio.h>
#include <json-glib/json-glib.h>
#define MAX_NATIVE_COMMAND_ARGUMENTS 64
#define MAX_NATIVE_COMMAND_BYTES (64 * 1024)
typedef struct { gboolean stopping; GHashTable *processes; } GnoblinNativeControl;
gboolean gnoblin_control_queue_command(const char *, const char *, char **, GError **);
void gnoblin_control_launch_command(const char *, const char *, char **);
gboolean gnoblin_control_reset_command_scheduling(void);
gboolean gnoblin_control_process_spawn(GnoblinNativeControl *, gint64, GVariant *, GError **);
void gnoblin_control_process_init(GnoblinNativeControl *);
void gnoblin_control_process_clear(GnoblinNativeControl *);
const char *gnoblin_control_operation_error_code(const GError *);
void gnoblin_control_dispatch_operation_completion_full(
    GnoblinNativeControl *, gint64, const char *, gboolean, JsonNode *,
    const char *, const char *, gboolean);
typedef enum {
    GNOBLIN_NATIVE_NOTICE_CONFIG_RELOADED,
    GNOBLIN_NATIVE_NOTICE_CONFIG_FAILED,
    GNOBLIN_NATIVE_NOTICE_COMMAND_FAILED,
} GnoblinNativeNoticeKind;
void gnoblin_control_post_notice(GnoblinNativeNoticeKind, const char *);
"""

PRIVATE_HEADER = r"""
#pragma once
gboolean native_runtime_dispatch_event(GnoblinNativeControl *, const char *, GVariant *);
"""

HARNESS = r"""
#define _GNU_SOURCE
#include "core/gnoblin-control-internal.h"
#include <sched.h>
#include <stdio.h>
#include <sys/resource.h>
#include <unistd.h>

static gboolean completed;
static int captured_nice;
static int captured_policy;
static int shared_nice;

const char *gnoblin_control_operation_error_code(const GError *error) {
    return error ? "error" : NULL;
}

static char *posted_notice;
static int posted_kind = -1;

void gnoblin_control_post_notice(GnoblinNativeNoticeKind kind, const char *message) {
    posted_kind = kind;
    g_free(posted_notice);
    posted_notice = g_strdup(message);
}

void gnoblin_control_dispatch_operation_completion_full(
    GnoblinNativeControl *control, gint64 id, const char *method, gboolean success,
    JsonNode *result, const char *code, const char *message, gboolean retry) {
    g_assert_cmpstr(method, ==, "process.spawn");
    g_assert_true(success);
}

/* process.spawn reports the exit with its captured output as this runtime event. */
gboolean native_runtime_dispatch_event(GnoblinNativeControl *control, const char *event,
                                       GVariant *payload) {
    g_assert_cmpstr(event, ==, "gnoblin.process.exited");
    gint64 exit_code = -1;
    g_assert_true(g_variant_lookup(payload, "exit_code", "x", &exit_code));
    g_assert_cmpint(exit_code, ==, 0);
    g_autoptr(GVariant) out = g_variant_lookup_value(payload, "stdout", G_VARIANT_TYPE_BYTESTRING);
    g_assert_nonnull(out);
    gsize length = 0;
    const char *bytes = g_variant_get_fixed_array(out, &length, 1);
    g_autofree char *text = g_strndup(bytes, length);
    g_assert_cmpint(sscanf(text, "%d %d", &captured_nice, &captured_policy), ==, 2);
    completed = TRUE;
    return TRUE;
}

static void shared_task(GTask *task, gpointer source, gpointer data, GCancellable *cancel) {
    g_task_return_int(task, getpriority(PRIO_PROCESS, 0));
}

static void shared_done(GObject *source, GAsyncResult *result, gpointer data) {
    shared_nice = g_task_propagate_int(G_TASK(result), NULL);
    completed = TRUE;
}

static void wait_completed(void) {
    gint64 deadline = g_get_monotonic_time() + 5 * G_USEC_PER_SEC;
    while (!completed && g_get_monotonic_time() < deadline) {
        while (g_main_context_iteration(NULL, FALSE)) {}
        g_usleep(1000);
    }
    g_assert_true(completed);
}

static int sample_shared_worker(void) {
    completed = FALSE;
    g_autoptr(GTask) task = g_task_new(NULL, NULL, shared_done, NULL);
    g_task_run_in_thread(task, shared_task);
    wait_completed();
    return shared_nice;
}

static void wait_record(const char *record, int expected_nice, int expected_policy, const char *path) {
    gint64 deadline = g_get_monotonic_time() + 5 * G_USEC_PER_SEC;
    while (!g_file_test(record, G_FILE_TEST_EXISTS) && g_get_monotonic_time() < deadline) {
        while (g_main_context_iteration(NULL, FALSE)) {}
        g_usleep(1000);
    }
    g_autofree char *text = NULL;
    g_assert_true(g_file_get_contents(record, &text, NULL, NULL));
    unlink(record);
    int nice = 0, policy = -1;
    g_assert_cmpint(sscanf(text, "%d %d", &nice, &policy), ==, 2);
    if (nice != expected_nice || policy != expected_policy)
        g_error("%s: child nice %d policy %d, expected %d/%d", path, nice, policy, expected_nice,
                expected_policy);
}

int main(int argc, char **argv) {
    if (argc == 3 && g_str_equal(argv[1], "--child")) {
        g_autofree char *text = g_strdup_printf("%d %d\n", getpriority(PRIO_PROCESS, 0),
                                                sched_getscheduler(0));
        g_assert_true(g_file_set_contents(argv[2], text, -1, NULL));
        fputs(text, stdout);
        return 0;
    }
    if (argc == 2 && g_str_equal(argv[1], "--positive"))
        g_assert_cmpint(setpriority(PRIO_PROCESS, 0, 19), ==, 0);
    int parent_nice = getpriority(PRIO_PROCESS, 0);
    int parent_policy = sched_getscheduler(0);
    int expected = parent_nice < 0 ? 0 : parent_nice;
    int shared_before = sample_shared_worker();
    g_autofree char *directory = g_dir_make_tmp("gnoblin-command-nice-XXXXXX", NULL);
    g_assert_nonnull(directory);
    g_autofree char *record = g_build_filename(directory, "child", NULL);
    char *child_argv[] = {argv[0], "--child", record, NULL};
    GnoblinNativeControl control = {0};
    gnoblin_control_process_init(&control);
    for (int repeat = 0; repeat < 3; repeat++) {
        g_autoptr(GError) error = NULL;
        gnoblin_control_launch_command("shortcut", "priority", child_argv);
        wait_record(record, expected, parent_policy, "shortcut");
        g_assert_true(gnoblin_control_queue_command("command", "priority", child_argv, &error));
        wait_record(record, expected, parent_policy, "command.run");
        g_assert_true(gnoblin_control_queue_command("touchpad", "priority", child_argv, &error));
        wait_record(record, expected, parent_policy, "touchpad");
        GVariantBuilder builder;
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&builder, "{sv}", "argv", g_variant_new_strv((const char **)child_argv, -1));
        g_variant_builder_add(&builder, "{sv}", "stdout", g_variant_new_string("capture"));
        g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&builder));
        completed = FALSE;
        g_assert_true(gnoblin_control_process_spawn(&control, repeat + 1, arguments, &error));
        wait_completed();
        if (captured_nice != expected || captured_policy != parent_policy)
            g_error("process.spawn: child nice %d policy %d, expected %d/%d", captured_nice,
                    captured_policy, expected, parent_policy);
        unlink(record);
        g_assert_cmpint(getpriority(PRIO_PROCESS, 0), ==, parent_nice);
        g_assert_cmpint(sched_getscheduler(0), ==, parent_policy);
    }
    /* A shortcut whose command is gone reports its name and the reason, not the path. */
    char *missing_argv[] = {"/nonexistent/gnoblin-missing-command", NULL};
    gnoblin_control_launch_command("shortcut", "media-next", missing_argv);
    gint64 notice_deadline = g_get_monotonic_time() + 5 * G_USEC_PER_SEC;
    while (!posted_notice && g_get_monotonic_time() < notice_deadline) {
        while (g_main_context_iteration(NULL, FALSE)) {}
        g_usleep(1000);
    }
    g_assert_cmpint(posted_kind, ==, GNOBLIN_NATIVE_NOTICE_COMMAND_FAILED);
    g_assert_cmpstr(posted_notice, ==, "media-next could not start: No such file or directory");
    g_assert_cmpint(sample_shared_worker(), ==, shared_before);
    gnoblin_control_process_clear(&control);
    g_hash_table_unref(control.processes);
    rmdir(directory);
    g_print("PASS: command and process children nice %d; parent nice %d policy %d and shared worker unchanged\n",
            expected, parent_nice, parent_policy);
    if (parent_nice >= 0)
        g_print("NOT CHECKED: negative-nice inheritance; run this probe at nice -5 separately\n");
    return 0;
}
"""


def build(directory):
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "core").mkdir(exist_ok=True)
    (directory / "config.h").write_text("")
    (directory / "core/gnoblin-control-internal.h").write_text(HEADER)
    (directory / "core/gnoblin-native-control-private.h").write_text(PRIVATE_HEADER)
    source = directory / "probe.c"
    source.write_text(HARNESS)
    binary = directory / "command-priority-probe"
    flags = shlex.split(
        subprocess.check_output(["pkg-config", "--cflags", "--libs", "gio-2.0", "json-glib-1.0"], text=True)
    )
    subprocess.run(
        [
            "cc",
            "-std=c11",
            "-O2",
            "-Wall",
            "-Werror",
            "-I",
            str(directory),
            str(source),
            *map(str, SOURCES),
            "-o",
            str(binary),
            *flags,
        ],
        check=True,
    )
    for path in SOURCES:
        print(hashlib.sha256(path.read_bytes()).hexdigest(), path, flush=True)
    return binary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-only", type=Path, help="Keep the probe for a separate boosted-parent run")
    args = parser.parse_args()
    if args.build_only:
        print(build(args.build_only.resolve()))
        return
    with tempfile.TemporaryDirectory(prefix="gnoblin-command-priority-") as directory:
        binary = build(Path(directory))
        subprocess.run([str(binary)], check=True, timeout=20)
        subprocess.run([str(binary), "--positive"], check=True, timeout=20)


if __name__ == "__main__":
    main()
