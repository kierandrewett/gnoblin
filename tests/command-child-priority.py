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
SOURCES = [ROOT / "src/control" / name for name in ("gnoblin-control-command.c", "gnoblin-control-capture.c")]

HEADER = r"""
#pragma once
#include <gio/gio.h>
#include <json-glib/json-glib.h>
#define MAX_NATIVE_COMMAND_ARGUMENTS 64
#define MAX_NATIVE_COMMAND_BYTES (64 * 1024)
typedef struct { gboolean stopping; GHashTable *pending_captures; } GnoblinNativeControl;
gboolean gnoblin_control_queue_command(const char *, const char *, char **, GError **);
gboolean gnoblin_control_capture_begin(GnoblinNativeControl *, gint64, GVariant *, GError **);
void gnoblin_control_capture_init(GnoblinNativeControl *);
void gnoblin_control_capture_clear(GnoblinNativeControl *);
const char *gnoblin_control_operation_error_code(const GError *);
void gnoblin_control_dispatch_operation_completion_full(
    GnoblinNativeControl *, gint64, const char *, gboolean, JsonNode *,
    const char *, const char *, gboolean);
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
static int shared_nice;

const char *gnoblin_control_operation_error_code(const GError *error) {
    return error ? "error" : NULL;
}

void gnoblin_control_dispatch_operation_completion_full(
    GnoblinNativeControl *control, gint64 id, const char *method, gboolean success,
    JsonNode *result, const char *code, const char *message, gboolean retry) {
    g_assert_true(success);
    JsonObject *object = json_node_get_object(result);
    g_assert_cmpint(json_object_get_int_member(object, "exit_code"), ==, 0);
    captured_nice = atoi(json_object_get_string_member(object, "stdout"));
    completed = TRUE;
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

int main(int argc, char **argv) {
    if (argc == 3 && g_str_equal(argv[1], "--child")) {
        int nice = getpriority(PRIO_PROCESS, 0);
        g_autofree char *text = g_strdup_printf("%d\n", nice);
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
    gnoblin_control_capture_init(&control);
    for (int repeat = 0; repeat < 3; repeat++) {
        g_autoptr(GError) error = NULL;
        g_assert_true(gnoblin_control_queue_command("test", "priority", child_argv, &error));
        gint64 deadline = g_get_monotonic_time() + 5 * G_USEC_PER_SEC;
        while (!g_file_test(record, G_FILE_TEST_EXISTS) && g_get_monotonic_time() < deadline) {
            while (g_main_context_iteration(NULL, FALSE)) {}
            g_usleep(1000);
        }
        g_autofree char *text = NULL;
        g_assert_true(g_file_get_contents(record, &text, NULL, NULL));
        g_assert_cmpint(atoi(text), ==, expected);
        unlink(record);
        GVariantBuilder builder;
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&builder, "{sv}", "argv", g_variant_new_strv((const char **)child_argv, -1));
        g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&builder));
        completed = FALSE;
        g_assert_true(gnoblin_control_capture_begin(&control, repeat + 1, arguments, &error));
        wait_completed();
        g_assert_cmpint(captured_nice, ==, expected);
        unlink(record);
        g_assert_cmpint(getpriority(PRIO_PROCESS, 0), ==, parent_nice);
        g_assert_cmpint(sched_getscheduler(0), ==, parent_policy);
    }
    g_assert_cmpint(sample_shared_worker(), ==, shared_before);
    gnoblin_control_capture_clear(&control);
    g_hash_table_unref(control.pending_captures);
    rmdir(directory);
    g_print("PASS: run/capture children nice %d; parent nice %d policy %d and shared worker unchanged\n",
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
