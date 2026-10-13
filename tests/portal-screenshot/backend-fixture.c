/* SPDX-License-Identifier: LGPL-2.1-or-later */
/* Actual portal request implementation with only consent UI and pixel source
 * excluded. Native UI and real compositor pixels belong to the VM gate. */
#include <stdio.h>
#include <unistd.h>
#include "gnoblin-screenshot.c"
#include <glib/gstdio.h>

void __wrap_gdk_display_flush(GdkDisplay* display) {}
GxdpExternalWindow* gxdp_external_window_new_from_handle(const char* handle) {
    g_assert_not_reached();
}
void gxdp_external_window_set_parent_of(GxdpExternalWindow* window, GdkSurface* surface) {
    g_assert_not_reached();
}
GdkDisplay* __wrap_gdk_display_get_default(void) {
    return (GdkDisplay*)1;
}
GListModel* __wrap_gdk_display_get_monitors(GdkDisplay* display) {
    static GListStore* monitors;
    if (!monitors) {
        monitors = g_list_store_new(G_TYPE_OBJECT);
        GObject* monitor = g_object_new(G_TYPE_OBJECT, NULL);
        g_list_store_append(monitors, monitor);
        g_object_unref(monitor);
    }
    return G_LIST_MODEL(monitors);
}
void __wrap_gdk_monitor_get_geometry(GdkMonitor* monitor, GdkRectangle* rectangle) {
    *rectangle = (GdkRectangle){0, 0, 64, 32};
}
GnoblinPermission gnoblin_permission_for_request(GDBusMethodInvocation* invocation,
                                                 const char* capability, const char* app_id,
                                                 const char* handle) {
    g_assert_cmpstr(capability, ==, "screenshot");
    return (GnoblinPermission){.level = g_strcmp0(app_id, "deny") == 0
                                            ? GNOBLIN_PERMISSION_DENY
                                            : GNOBLIN_PERMISSION_DEFAULT};
}
static void discard(gpointer data) {
    g_unlink(data);
    g_free(data);
}
static gboolean complete_capture(gpointer data) {
    GTask* task = data;
    if (!g_task_return_error_if_cancelled(task)) {
        g_autofree char* file = NULL;
        int fd = g_file_open_tmp("gnoblin-screenshot-XXXXXX", &file, NULL);
        close(fd);
        g_autoptr(GdkPixbuf) image = gdk_pixbuf_new(GDK_COLORSPACE_RGB, FALSE, 8, 64, 32);
        gdk_pixbuf_fill(image, 0xff0000ff);
        g_assert_true(gdk_pixbuf_save(image, file, "png", NULL, NULL));
        g_task_return_pointer(task, g_steal_pointer(&file), discard);
    }
    g_print("CAPTURE-DONE\n");
    g_object_unref(task);
    return G_SOURCE_REMOVE;
}
void gnoblin_screenshot_capture_async(const GnoblinScreenshotArea* area, gboolean cursor,
                                      GCancellable* cancellable, GAsyncReadyCallback callback,
                                      gpointer data) {
    g_print("CAPTURE\n");
    GTask* task = g_task_new(NULL, cancellable, callback, data);
    g_timeout_add(200, complete_capture, task);
}
char* gnoblin_screenshot_capture_finish(GAsyncResult* result, GError** error) {
    return g_task_propagate_pointer(G_TASK(result), error);
}
static gboolean access_allowed(XdpImplAccess* impl, GDBusMethodInvocation* invocation,
                               const char* handle, const char* app_id, const char* parent,
                               const char* title, const char* subtitle, const char* body,
                               GVariant* options, gpointer data) {
    xdp_impl_access_complete_access_dialog(impl, invocation, 0,
                                           g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
    return TRUE;
}
int main(void) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    /* The frontend hint never substitutes for interactive consent or ask. */
    g_assert_false(screenshot_needs_consent(GNOBLIN_PERMISSION_DEFAULT, FALSE, TRUE));
    g_assert_true(screenshot_needs_consent(GNOBLIN_PERMISSION_DEFAULT, FALSE, FALSE));
    g_assert_true(screenshot_needs_consent(GNOBLIN_PERMISSION_ASK, FALSE, TRUE));
    g_assert_false(screenshot_needs_consent(GNOBLIN_PERMISSION_ALLOW, FALSE, FALSE));
    g_assert_true(screenshot_needs_consent(GNOBLIN_PERMISSION_ALLOW, TRUE, TRUE));
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    g_assert_no_error(error);
    XdpImplAccess* access = xdp_impl_access_skeleton_new();
    g_signal_connect(access, "handle-access-dialog", G_CALLBACK(access_allowed), NULL);
    g_assert_true(g_dbus_interface_skeleton_export(G_DBUS_INTERFACE_SKELETON(access), bus,
                                                   DESKTOP_PORTAL_OBJECT_PATH, &error));
    g_assert_no_error(error);
    g_assert_true(screenshot_init(bus, &error));
    g_assert_no_error(error);
    g_bus_own_name_on_connection(bus, "org.freedesktop.impl.portal.desktop.gnoblin",
                                 G_BUS_NAME_OWNER_FLAGS_NONE, NULL, NULL, NULL, NULL);
    g_print("READY\n");
    GMainLoop* loop = g_main_loop_new(NULL, FALSE);
    g_main_loop_run(loop);
    return 0;
}
