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
        (void)syscall(SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0,
                      IOPRIO_PRIO_VALUE(IOPRIO_CLASS_BE, 4));
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
