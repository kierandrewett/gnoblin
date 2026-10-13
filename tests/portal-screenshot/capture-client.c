/* SPDX-License-Identifier: LGPL-2.1-or-later */
#include "gnoblin-screenshot-capture.h"
#include <glib/gstdio.h>
#include <sys/stat.h>

static GMainLoop* loop;
static int status = 1;
static void done(GObject* object, GAsyncResult* result, gpointer user_data) {
    g_autoptr(GError) error = NULL;
    g_autofree char* file = gnoblin_screenshot_capture_finish(result, &error);
    if (!file) {
        g_print("ERROR %s\n", error->message);
    } else {
        struct stat st;
        g_assert_cmpint(g_stat(file, &st), ==, 0);
        g_assert_cmpint(st.st_mode & 0777, ==, 0600);
        g_autoptr(GdkPixbuf) image = gdk_pixbuf_new_from_file(file, &error);
        g_assert_no_error(error);
        if (g_getenv("GNOBLIN_TEST_RED")) {
            const guchar* pixel = gdk_pixbuf_get_pixels(image);
            g_assert_cmpint(pixel[0], ==, 255);
            g_assert_cmpint(pixel[1], ==, 0);
            g_assert_cmpint(pixel[2], ==, 0);
        }
        g_print("PNG %d %d %s\n", gdk_pixbuf_get_width(image), gdk_pixbuf_get_height(image), file);
        g_unlink(file);
        status = 0;
    }
    g_main_loop_quit(loop);
}
static gboolean cancel(gpointer data) {
    g_cancellable_cancel(data);
    return G_SOURCE_REMOVE;
}
int main(int argc, char** argv) {
    g_autoptr(GCancellable) cancellable = g_cancellable_new();
    loop = g_main_loop_new(NULL, FALSE);
    GnoblinScreenshotArea area = {.x = -32, .y = 0, .width = 64, .height = 32};
    gnoblin_screenshot_capture_async(&area, FALSE, cancellable, done, NULL);
    if (argc == 2)
        g_timeout_add((guint)g_ascii_strtoull(argv[1], NULL, 10), cancel, cancellable);
    g_main_loop_run(loop);
    g_main_loop_unref(loop);
    return status;
}
