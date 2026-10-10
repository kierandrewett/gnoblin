/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "gnoblin-priority.h"

#include <gio/gio.h>
#include <errno.h>
#include <linux/ioprio.h>
#include <sys/resource.h>
#include <sched.h>
#include <sys/syscall.h>
#include <unistd.h>

static int io_priority(void) {
#ifdef SYS_ioprio_get
    return (int)syscall(SYS_ioprio_get, IOPRIO_WHO_PROCESS, 0);
#else
    return -1;
#endif
}

static int scheduling_policy(void) {
    int policy = sched_getscheduler(0);
#ifdef SCHED_RESET_ON_FORK
    if (policy >= 0)
        policy &= ~SCHED_RESET_ON_FORK;
#endif
    return policy;
}

static void restore_io_priority(void) {
#ifdef SYS_ioprio_set
    if (io_priority() >= 0 && IOPRIO_PRIO_CLASS(io_priority()) == IOPRIO_CLASS_IDLE)
        (void)syscall(SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, IOPRIO_PRIO_VALUE(IOPRIO_CLASS_BE, 4));
#endif
}

static gboolean request_rtkit_priority(void) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) bus = g_bus_get_sync(G_BUS_TYPE_SYSTEM, NULL, &error);
    if (!bus)
        return FALSE;
    /* RealtimeKit's MinNiceLevel is normally -11. Requesting that value also
     * works with its policy clamp and lets unprivileged sessions escape a
     * positive nice inherited from gdm. */
    g_autoptr(GVariant) result = g_dbus_connection_call_sync(
        bus, "org.freedesktop.RealtimeKit1", "/org/freedesktop/RealtimeKit1",
        "org.freedesktop.RealtimeKit1", "MakeThreadHighPriority",
        g_variant_new("(ti)", (guint64)syscall(SYS_gettid), -11), NULL,
        G_DBUS_CALL_FLAGS_NO_AUTO_START, 1000, NULL, &error);
    return result != NULL;
}

void gnoblin_restore_interactive_priority(const char* component) {
    int before_policy = scheduling_policy();
    errno = 0;
    int before_nice = getpriority(PRIO_PROCESS, 0);
    int before_io = io_priority();
    gboolean needs_rtkit = FALSE;

    if (before_policy == SCHED_IDLE || before_policy == SCHED_BATCH) {
        struct sched_param param = {0};
        if (sched_setscheduler(0, SCHED_OTHER, &param) != 0)
            needs_rtkit = TRUE;
    }
    if (before_nice > 0 && setpriority(PRIO_PROCESS, 0, 0) != 0)
        needs_rtkit = TRUE;
    restore_io_priority();

    int after_policy = scheduling_policy();
    errno = 0;
    int after_nice = getpriority(PRIO_PROCESS, 0);
    int after_io = io_priority();
    if (after_policy == SCHED_IDLE || after_policy == SCHED_BATCH || after_nice > 0 ||
        (after_io >= 0 && IOPRIO_PRIO_CLASS(after_io) == IOPRIO_CLASS_IDLE))
        needs_rtkit = TRUE;
    if (needs_rtkit)
        (void)request_rtkit_priority();

    g_message("gnoblin-%s: scheduling policy %d->%d, nice %d->%d, io %d->%d%s", component,
              before_policy, scheduling_policy(), before_nice, getpriority(PRIO_PROCESS, 0),
              before_io, io_priority(), needs_rtkit ? " (rtkit requested)" : "");
}
/* ---- Ananicy notice ---------------------------------------------------- */

/* Ananicy-cpp's default rules rank gdm and its children as background work, so a
 * session started by gdm can inherit SCHED_IDLE and freeze or drop frames under
 * load. A process cannot leave that class unprivileged. When the class is still
 * degraded after the reset above, Ananicy is installed and the Gnoblin rules are
 * not, tell the user once per login how to fix it. */
#define ANANICY_RULES_FILE "/etc/ananicy.d/99-gnoblin.rules"
#define NOTIFICATIONS_NAME "org.freedesktop.Notifications"
#define NOTICE_DISMISSED_FILE "ananicy-notice-dismissed"
#define NOTICE_URL                                                                                 \
    "https://github.com/kierandrewett/gnoblin/blob/main/docs/troubleshooting.md#ananicy-cpp-rules"

static struct {
    guint watch_id;
    guint signal_id;
    guint32 notification_id;
    gboolean sent;
} priority_notice;

static gboolean priority_is_degraded(void) {
    int policy = scheduling_policy();
    errno = 0;
    int nice_value = getpriority(PRIO_PROCESS, 0);
    int io = io_priority();
    return policy == SCHED_IDLE || policy == SCHED_BATCH || nice_value > 0 ||
           (io >= 0 && IOPRIO_PRIO_CLASS(io) == IOPRIO_CLASS_IDLE);
}

static gboolean ananicy_is_installed(void) {
    g_autofree char* daemon = g_find_program_in_path("ananicy-cpp");
    g_autofree char* legacy = g_find_program_in_path("ananicy");
    return daemon || legacy || g_file_test("/etc/ananicy.d", G_FILE_TEST_IS_DIR);
}

static char* notice_state_path(void) {
    return g_build_filename(g_get_user_state_dir(), "gnoblin", NOTICE_DISMISSED_FILE, NULL);
}

static char* notice_shown_path(void) {
    return g_build_filename(g_get_user_runtime_dir(), "gnoblin-ananicy-notice-shown", NULL);
}

static void priority_notice_action(GDBusConnection* connection, const char* sender,
                                   const char* object_path, const char* interface_name,
                                   const char* signal_name, GVariant* parameters,
                                   gpointer user_data) {
    guint32 id = 0;
    const char* action = NULL;
    g_variant_get(parameters, "(u&s)", &id, &action);
    if (id != priority_notice.notification_id || !g_str_equal(action, "dismiss"))
        return;
    g_autofree char* path = notice_state_path();
    g_autofree char* directory = g_path_get_dirname(path);
    g_mkdir_with_parents(directory, 0700);
    g_file_set_contents(path, "", 0, NULL);
}

static void priority_notice_name_appeared(GDBusConnection* bus, const char* name, const char* owner,
                                          gpointer user_data) {
    if (priority_notice.sent)
        return;
    priority_notice.sent = TRUE;
    priority_notice.signal_id = g_dbus_connection_signal_subscribe(
        bus, owner, "org.freedesktop.Notifications", "ActionInvoked",
        "/org/freedesktop/Notifications", NULL, G_DBUS_SIGNAL_FLAGS_NONE, priority_notice_action,
        NULL, NULL);

    g_autoptr(GVariantBuilder) hints = g_variant_builder_new(G_VARIANT_TYPE("a{sv}"));
    g_variant_builder_add(hints, "{sv}", "urgency", g_variant_new_byte(1));
    g_variant_builder_add(hints, "{sv}", "category", g_variant_new_string("device"));
    const char* actions[] = {"dismiss", "Don't show again", NULL};
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) result = g_dbus_connection_call_sync(
        bus, owner, "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "Notify",
        g_variant_new(
            "(susssasa{sv}i)", "Gnoblin", (guint32)0, "dialog-warning-symbolic",
            "Gnoblin is running at background priority",
            "Ananicy is installed, and its default rules give this session idle priority. "
            "Videos and the desktop can drop frames or freeze while other programs use the CPU. "
            "Install the Gnoblin Ananicy rules to fix it, then log out and back in. " NOTICE_URL,
            actions, hints, 0),
        G_VARIANT_TYPE("(u)"), G_DBUS_CALL_FLAGS_NONE, 2000, NULL, &error);
    if (!result) {
        g_message("gnoblin: could not send the priority notice: %s", error->message);
        return;
    }
    g_variant_get(result, "(u)", &priority_notice.notification_id);
    g_autofree char* shown = notice_shown_path();
    g_file_set_contents(shown, "", 0, NULL);
}

void gnoblin_priority_notice_start(void) {
    /* GNOBLIN_PRIORITY_NOTICE=never turns the notice off. "always" skips the checks, for tests. */
    const char* mode = g_getenv("GNOBLIN_PRIORITY_NOTICE");
    if (g_strcmp0(mode, "never") == 0 || priority_notice.watch_id)
        return;
    if (g_strcmp0(mode, "always") != 0 && (!priority_is_degraded() || !ananicy_is_installed() ||
                                           g_file_test(ANANICY_RULES_FILE, G_FILE_TEST_EXISTS)))
        return;
    g_message("gnoblin: the session runs at background priority and Ananicy is installed without "
              "the Gnoblin rules; see " NOTICE_URL);
    g_autofree char* dismissed = notice_state_path();
    g_autofree char* shown = notice_shown_path();
    if (g_file_test(dismissed, G_FILE_TEST_EXISTS) || g_file_test(shown, G_FILE_TEST_EXISTS))
        return;
    priority_notice.watch_id =
        g_bus_watch_name(G_BUS_TYPE_SESSION, NOTIFICATIONS_NAME, G_BUS_NAME_WATCHER_FLAGS_NONE,
                         priority_notice_name_appeared, NULL, NULL, NULL);
}
