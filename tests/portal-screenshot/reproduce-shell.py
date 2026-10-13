#!/usr/bin/env python3
"""Run the pre-fix screenshot transport and callback against an empty private bus.

Extract only the two transport functions from the patched upstream source;
replace UI presentation with an observation, so no compositor or GUI is needed.
A missing Shell service must complete as failure, rather than abandon the request.
"""

import os
from pathlib import Path
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "subprojects/xdg-desktop-portal-gnome/src/screenshotdialog.c"


def function(source, name):
    start = source.index("static ", source.rfind("\n}", 0, source.index("\n" + name + " (")) + 2)
    body = source.index("{", source.index("\n" + name + " ("))
    end = source.index("\n}", body) + 2
    return source[start:end]


def main():
    with tempfile.TemporaryDirectory(prefix="gnoblin-142-red-") as directory:
        build = Path(directory)
        subprocess.run(
            [
                "gdbus-codegen",
                "--generate-c-code",
                str(build / "shell-dbus"),
                str(ROOT / "subprojects/xdg-desktop-portal-gnome/data/org.gnome.Shell.Screenshot.xml"),
            ],
            check=True,
        )
        original = SOURCE.read_text()
        code = r"""
#include <gio/gio.h>
#include <stdio.h>
#include <string.h>
#include "shell-dbus.h"
typedef struct {
    OrgGnomeShellScreenshot *shell;
    GActionGroup *actions;
    GCancellable *cancellable;
    guint timeout;
} ScreenshotDialog;
static gboolean completed;
static void maybe_show_screenshot(ScreenshotDialog *d, const char *name) { completed = TRUE; }
static void screenshot_window_done(GObject *o, GAsyncResult *r, gpointer d) { g_assert_not_reached(); }
"""
        code += function(original, "screenshot_done") + "\n"
        code += function(original, "do_take_screenshot") + "\n"
        code += r"""
int main(void) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    g_assert_no_error(error);
    ScreenshotDialog dialog = {0};
    dialog.shell = org_gnome_shell_screenshot_proxy_new_sync(bus, G_DBUS_PROXY_FLAGS_DO_NOT_AUTO_START,
        "org.gnome.Shell.Screenshot", "/org/gnome/Shell/Screenshot", NULL, &error);
    g_assert_no_error(error);
    dialog.cancellable = g_cancellable_new();
    dialog.actions = G_ACTION_GROUP(g_simple_action_group_new());
    const char *names[] = {"grab", "pointer", "border"};
    for (int i = 0; i < 3; i++) {
        GSimpleAction *a = g_simple_action_new_stateful(names[i], NULL,
            i == 0 ? g_variant_new_string("screen") : g_variant_new_boolean(FALSE));
        g_action_map_add_action(G_ACTION_MAP(dialog.actions), G_ACTION(a));
        g_object_unref(a);
    }
    do_take_screenshot(&dialog);
    gint64 end = g_get_monotonic_time() + G_TIME_SPAN_SECOND;
    while (!completed && g_get_monotonic_time() < end) {
        while (g_main_context_iteration(NULL, FALSE));
        g_usleep(1000);
    }
    g_object_unref(dialog.actions);
    g_object_unref(dialog.cancellable);
    g_object_unref(dialog.shell);
    if (!completed) {
        g_printerr("FAIL #142: real screenshot transport reached absent Shell; request never completed\n");
        return 1;
    }
    return 0;
}
"""
        (build / "repro.c").write_text(code)
        flags = shlex.split(subprocess.check_output(["pkg-config", "--cflags", "--libs", "gio-2.0"], text=True))
        subprocess.run(
            [
                os.environ.get("CC", "cc"),
                "-g",
                "-I" + directory,
                str(build / "repro.c"),
                str(build / "shell-dbus.c"),
                *flags,
                "-o",
                str(build / "repro"),
            ],
            check=True,
        )
        return subprocess.run(["dbus-run-session", "--", str(build / "repro")]).returncode


if __name__ == "__main__":
    raise SystemExit(main())
