/* SPDX-License-Identifier: GPL-2.0-or-later
 * Non-GUI GSubprocess fixture. run.py supplies source excerpts, not a copy of
 * the production lifetime callback. All event writes fit within PIPE_BUF.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <gio/gio.h>
#include <pthread.h>
#include <signal.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <unistd.h>

enum {
    ENTER = 1,
    INSTALLED,
    EXECUTED,
    SPAWNED,
    FAILED,
    THREAD_DONE,
    NAME_ACQUIRED,
    ENABLED,
    SPAWNING,
    DIED
};
static int event_fd, gate_fd;
static const char* mode;
static GSpawnChildSetupFunc production_setup;
static gpointer production_data;
static GSubprocess* viewer;
static GMainLoop* loop;
static guint enabled_signal;
static unsigned int acquired_names;

static void event(int kind, int value) {
    int record[] = {kind, getpid(), gettid(), getppid(), value};
    ssize_t result;

    do
        result = write(event_fd, record, sizeof record);
    while (result < 0 && errno == EINTR);
    if (result != sizeof record)
        _exit(120);
}

static void gate(void) {
    char byte;
    ssize_t result;

    do
        result = read(gate_fd, &byte, 1);
    while (result < 0 && errno == EINTR);
    if (result != 1)
        _exit(121);
}

static void instrumented_setup(gpointer unused) {
    (void)unused;
    event(ENTER, 0);
    if (strcmp(mode, "before") == 0)
        gate();
    if (production_setup)
        production_setup(production_data);
    event(INSTALLED, 0);
    if (strcmp(mode, "after") == 0)
        gate();
}

/* Interpose only at the test seam: retain the exact production registration
 * arguments and wrap its callback with deterministic barriers. */
static void G_GNUC_UNUSED record_setup(GSubprocessLauncher* launcher, GSpawnChildSetupFunc callback,
                                       gpointer data, GDestroyNotify destroy) {
    (void)launcher;
    g_assert_null(destroy);
    production_setup = callback;
    production_data = data;
}

static int G_GNUC_UNUSED fixture_prctl(int option, int argument) {
    if (strcmp(mode, "prctl-failure") == 0) {
        errno = EPERM;
        return -1;
    }
    return prctl(option, argument);
}

#define prctl fixture_prctl
#define g_subprocess_launcher_set_child_setup record_setup
#include "spawn-under-test.h"
#undef g_subprocess_launcher_set_child_setup
#undef prctl

static void* spawn_viewer(void* unused) {
    g_autoptr(GSubprocessLauncher) launcher = launcher_under_test();
    g_autoptr(GError) error = NULL;
    const char* args[] = {"/proc/self/exe", "viewer", NULL};

    (void)unused;
    event(SPAWNING, 0);
    if (strcmp(mode, "spawn-failure") == 0)
        args[0] = "/no/such/gnoblin-devkit-lifecycle-viewer";
    g_subprocess_launcher_take_fd(launcher, dup(event_fd), event_fd);
    g_subprocess_launcher_take_fd(launcher, dup(gate_fd), gate_fd);
    g_subprocess_launcher_set_child_setup(launcher, instrumented_setup, NULL, NULL);
    viewer = g_subprocess_launcher_spawnv(launcher, args, &error);
    if (!viewer) {
        g_assert_nonnull(error);
        event(FAILED, 0);
    } else
        event(SPAWNED, atoi(g_subprocess_get_identifier(viewer)));
    if (strcmp(mode, "thread") == 0)
        gate();
    return NULL;
}

static void on_enabled(GObject* object, gpointer unused) {
    (void)object;
    (void)unused;
    event(ENABLED, 0);
    if (++acquired_names == 2) {
        spawn_viewer(NULL);
        g_main_loop_quit(loop);
    }
}

static void on_name_acquired(GDBusConnection* connection, const char* name, gpointer object) {
    (void)connection;
    (void)name;
    event(NAME_ACQUIRED, 0);
    g_signal_emit(object, enabled_signal, 0);
}

int main(int argc, char** argv) {
    char command;
    const char* fd = getenv("FIXTURE_EVENT_FD");
    const char* gate_number = getenv("FIXTURE_GATE_FD");
    guint names[2] = {0};
    g_autoptr(GObject) object = NULL;

    if (argc < 2 || !fd || !gate_number)
        return 2;
    event_fd = atoi(fd);
    gate_fd = atoi(gate_number);
    signal(SIGTERM, SIG_DFL);
    if (strcmp(argv[1], "viewer") == 0) {
        event(EXECUTED, 0);
        for (;;)
            pause();
    }
    mode = argv[1];
    if (strcmp(mode, "context") == 0) {
        /* Same GLib dispatch pattern as the two MetaDBusSessionManager enabled
         * signals. Only the runner's disposable bus is reachable here. */
        object = g_object_new(G_TYPE_OBJECT, NULL);
        enabled_signal = g_signal_new("fixture-enabled", G_TYPE_OBJECT, G_SIGNAL_RUN_LAST, 0, NULL,
                                      NULL, NULL, G_TYPE_NONE, 0);
        g_signal_connect(object, "fixture-enabled", G_CALLBACK(on_enabled), NULL);
        loop = g_main_loop_new(NULL, FALSE);
        names[0] =
            g_bus_own_name(G_BUS_TYPE_SESSION, "org.gnoblin.Fixture.Remote",
                           G_BUS_NAME_OWNER_FLAGS_NONE, NULL, on_name_acquired, NULL, object, NULL);
        names[1] =
            g_bus_own_name(G_BUS_TYPE_SESSION, "org.gnoblin.Fixture.Cast",
                           G_BUS_NAME_OWNER_FLAGS_NONE, NULL, on_name_acquired, NULL, object, NULL);
        g_main_loop_run(loop);
    } else if (strcmp(mode, "thread") == 0) {
        pthread_t thread;

        if (pthread_create(&thread, NULL, spawn_viewer, NULL) != 0 ||
            pthread_join(thread, NULL) != 0)
            return 2;
        event(THREAD_DONE, 0);
        if (!viewer || !g_subprocess_wait(viewer, NULL, NULL))
            return 2;
        event(DIED, g_subprocess_get_if_signaled(viewer) ? g_subprocess_get_term_sig(viewer) : 0);
    } else
        spawn_viewer(NULL);

    if (strcmp(mode, "prctl-failure") == 0 && viewer) {
        if (!g_subprocess_wait(viewer, NULL, NULL))
            return 2;
        event(DIED, g_subprocess_get_if_exited(viewer) ? g_subprocess_get_exit_status(viewer) : -1);
    }

    /* Keep the owner process/main thread alive until the runner ends the case. */
    if (read(STDIN_FILENO, &command, 1) != 1)
        return 2;
    if (names[0]) {
        g_bus_unown_name(names[0]);
        g_bus_unown_name(names[1]);
        g_main_loop_unref(loop);
    }
    g_clear_object(&viewer);
    return 0;
}
