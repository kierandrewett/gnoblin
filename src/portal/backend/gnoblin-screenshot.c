/* SPDX-License-Identifier: LGPL-2.1-or-later */
/* Screenshot portal for Gnoblin. Pixel capture is a short-lived Mutter
 * ScreenCast session; this backend does not depend on GNOME Shell. */
#include "config.h"

#include <gtk/gtk.h>
#include <gxdp.h>
#include <gio/gdesktopappinfo.h>
#include <glib/gstdio.h>
#include <glib/gi18n.h>

#include "gnoblin-screenshot-capture.h"
#include "request.h"
#include "screenshot.h"
#include "utils.h"

typedef struct {
    guint refs;
    XdpImplScreenshot* impl;
    GDBusMethodInvocation* invocation;
    Request* request;
    GCancellable* cancellable;
    GtkWidget* dialog;
    GtkWidget* pointer;
    GxdpExternalWindow* external_parent;
    guint frontend_watch;
    gboolean completed;
} Screenshot;

static Screenshot* screenshot_ref(Screenshot* shot) {
    shot->refs++;
    return shot;
}

static void screenshot_unref(Screenshot* shot) {
    if (--shot->refs)
        return;
    g_clear_object(&shot->impl);
    g_clear_object(&shot->invocation);
    g_clear_object(&shot->request);
    g_clear_object(&shot->cancellable);
    g_clear_object(&shot->external_parent);
    g_free(shot);
}

static void destroy_dialog(Screenshot* shot) {
    if (!shot->dialog)
        return;
    g_signal_handlers_disconnect_by_data(shot->dialog, shot);
    gtk_window_destroy(GTK_WINDOW(shot->dialog));
    g_clear_object(&shot->dialog);
    shot->pointer = NULL;
}

static void screenshot_complete(Screenshot* shot, guint response, const char* filename) {
    GVariantBuilder results;
    g_autofree char* uri = NULL;
    if (shot->completed)
        return;
    shot->completed = TRUE;
    g_cancellable_cancel(shot->cancellable);
    if (shot->frontend_watch) {
        g_bus_unwatch_name(shot->frontend_watch);
        shot->frontend_watch = 0;
    }
    g_signal_handlers_disconnect_by_data(shot->request, shot);
    if (shot->request->exported)
        request_unexport(shot->request);
    destroy_dialog(shot);
    g_variant_builder_init(&results, G_VARIANT_TYPE_VARDICT);
    if (response == 0) {
        uri = g_filename_to_uri(filename, NULL, NULL);
        if (uri)
            g_variant_builder_add(&results, "{sv}", "uri", g_variant_new_string(uri));
        else {
            g_unlink(filename);
            response = 2;
        }
    }
    xdp_impl_screenshot_complete_screenshot(shot->impl, shot->invocation, response,
                                            g_variant_builder_end(&results));
    screenshot_unref(shot);
}

static gboolean handle_close(XdpImplRequest* request, GDBusMethodInvocation* invocation,
                             Screenshot* shot) {
    if (g_strcmp0(g_dbus_method_invocation_get_sender(invocation), shot->request->sender) != 0) {
        g_dbus_method_invocation_return_error_literal(invocation, G_DBUS_ERROR,
                                                      G_DBUS_ERROR_ACCESS_DENIED,
                                                      "Only the request owner may close it");
        return TRUE;
    }
    xdp_impl_request_complete_close(request, invocation);
    screenshot_complete(shot, 1, NULL);
    return TRUE;
}

static void frontend_vanished(GDBusConnection* bus, const char* name, gpointer data) {
    screenshot_complete(data, 2, NULL);
}

static gboolean desktop_area(GnoblinScreenshotArea* area) {
    GdkDisplay* display = gdk_display_get_default();
    if (!display)
        return FALSE;
    GListModel* monitors = gdk_display_get_monitors(display);
    int64_t left = G_MAXINT, top = G_MAXINT, right = G_MININT, bottom = G_MININT;
    for (guint i = 0; i < g_list_model_get_n_items(monitors); i++) {
        g_autoptr(GdkMonitor) monitor = g_list_model_get_item(monitors, i);
        GdkRectangle rectangle;
        gdk_monitor_get_geometry(monitor, &rectangle);
        if (rectangle.width <= 0 || rectangle.height <= 0)
            return FALSE;
        left = MIN(left, rectangle.x);
        top = MIN(top, rectangle.y);
        right = MAX(right, (int64_t)rectangle.x + rectangle.width);
        bottom = MAX(bottom, (int64_t)rectangle.y + rectangle.height);
    }
    if (right <= left || bottom <= top || right > G_MAXINT || bottom > G_MAXINT ||
        right - left > G_MAXINT || bottom - top > G_MAXINT)
        return FALSE;
    *area = (GnoblinScreenshotArea){(int)left, (int)top, (int)(right - left), (int)(bottom - top)};
    return TRUE;
}

static void capture_done(GObject* object, GAsyncResult* result, gpointer data) {
    Screenshot* shot = data;
    g_autoptr(GError) error = NULL;
    g_autofree char* filename = gnoblin_screenshot_capture_finish(result, &error);
    if (shot->completed) {
        if (filename)
            g_unlink(filename);
    } else if (!filename) {
        g_debug("Screenshot failed: %s", error->message);
        screenshot_complete(shot, g_error_matches(error, G_IO_ERROR, G_IO_ERROR_CANCELLED) ? 1 : 2,
                            NULL);
    } else {
        screenshot_complete(shot, 0, filename);
    }
    screenshot_unref(shot);
}

static void take_screenshot(Screenshot* shot, gboolean include_cursor) {
    GnoblinScreenshotArea area;
    destroy_dialog(shot);
    if (!desktop_area(&area)) {
        screenshot_complete(shot, 2, NULL);
        return;
    }
    /* Send the consent window's unmap before asking the compositor to paint. */
    gdk_display_flush(gdk_display_get_default());
    gnoblin_screenshot_capture_async(&area, include_cursor, shot->cancellable, capture_done,
                                     screenshot_ref(shot));
}

static void dialog_response(GtkDialog* dialog, int response, gpointer data) {
    Screenshot* shot = data;
    if (response != GTK_RESPONSE_ACCEPT) {
        screenshot_complete(shot, 1, NULL);
        return;
    }
    gboolean pointer = gtk_check_button_get_active(GTK_CHECK_BUTTON(shot->pointer));
    take_screenshot(shot, pointer);
}

static void show_consent(Screenshot* shot, const char* app_id, const char* parent_window,
                         gboolean modal) {
    g_autofree char* desktop_id = *app_id ? g_strconcat(app_id, ".desktop", NULL) : NULL;
    g_autoptr(GDesktopAppInfo) info = desktop_id ? g_desktop_app_info_new(desktop_id) : NULL;
    g_autofree char* message =
        info ? g_strdup_printf(_("Allow %s to take a screenshot of the entire desktop?"),
                               g_app_info_get_display_name(G_APP_INFO(info)))
             : g_strdup(_("Allow the requesting app to take a screenshot of the entire desktop?"));
    shot->dialog = g_object_ref_sink(gtk_dialog_new_with_buttons(
        _("Take a Screenshot?"), NULL, modal ? GTK_DIALOG_MODAL : 0, _("Cancel"),
        GTK_RESPONSE_CANCEL, _("Take Screenshot"), GTK_RESPONSE_ACCEPT, NULL));
    gtk_dialog_set_default_response(GTK_DIALOG(shot->dialog), GTK_RESPONSE_CANCEL);
    GtkWidget* box = gtk_dialog_get_content_area(GTK_DIALOG(shot->dialog));
    gtk_box_set_spacing(GTK_BOX(box), 16);
    gtk_widget_set_margin_top(box, 24);
    gtk_widget_set_margin_bottom(box, 24);
    gtk_widget_set_margin_start(box, 24);
    gtk_widget_set_margin_end(box, 24);
    GtkWidget* label = gtk_label_new(message);
    gtk_label_set_wrap(GTK_LABEL(label), TRUE);
    gtk_label_set_max_width_chars(GTK_LABEL(label), 50);
    gtk_box_append(GTK_BOX(box), label);
    shot->pointer = gtk_check_button_new_with_label(_("Include pointer"));
    gtk_box_append(GTK_BOX(box), shot->pointer);
    if (parent_window && *parent_window)
        shot->external_parent = gxdp_external_window_new_from_handle(parent_window);
    gtk_widget_realize(shot->dialog);
    if (shot->external_parent)
        gxdp_external_window_set_parent_of(shot->external_parent,
                                           gtk_native_get_surface(GTK_NATIVE(shot->dialog)));
    g_signal_connect(shot->dialog, "response", G_CALLBACK(dialog_response), shot);
    gtk_window_present(GTK_WINDOW(shot->dialog));
}

static gboolean handle_screenshot(XdpImplScreenshot* impl, GDBusMethodInvocation* invocation,
                                  const char* handle, const char* app_id, const char* parent_window,
                                  GVariant* options) {
    /* Every request shows the consent dialog. No policy or frontend hint skips it. */
    gboolean modal = TRUE;
    g_variant_lookup(options, "modal", "b", &modal);
    Screenshot* shot = g_new0(Screenshot, 1);
    shot->refs = 1;
    shot->impl = g_object_ref(impl);
    shot->invocation = g_object_ref(invocation);
    shot->request = request_new(g_dbus_method_invocation_get_sender(invocation), app_id, handle);
    shot->cancellable = g_cancellable_new();
    g_signal_connect(shot->request, "handle-close", G_CALLBACK(handle_close), shot);
    request_export(shot->request, g_dbus_method_invocation_get_connection(invocation));
    shot->frontend_watch = g_bus_watch_name_on_connection(
        g_dbus_method_invocation_get_connection(invocation), "org.freedesktop.portal.Desktop",
        G_BUS_NAME_WATCHER_FLAGS_NONE, NULL, frontend_vanished, shot, NULL);
    show_consent(shot, app_id, parent_window, modal);
    return TRUE;
}

static gboolean handle_pick_color(XdpImplScreenshot* impl, GDBusMethodInvocation* invocation,
                                  const char* handle, const char* app_id, const char* parent_window,
                                  GVariant* options) {
    /* Picking a pixel requires a selection UI. Until there is a native one,
     * fail promptly, with no capture and no dependency on GNOME Shell. */
    xdp_impl_screenshot_complete_pick_color(impl, invocation, 2,
                                            g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
    return TRUE;
}

gboolean screenshot_init(GDBusConnection* bus, GError** error) {
    XdpImplScreenshot* impl = xdp_impl_screenshot_skeleton_new();
    xdp_impl_screenshot_set_version(impl, 2);
    g_signal_connect(impl, "handle-screenshot", G_CALLBACK(handle_screenshot), NULL);
    g_signal_connect(impl, "handle-pick-color", G_CALLBACK(handle_pick_color), NULL);
    if (!g_dbus_interface_skeleton_export(G_DBUS_INTERFACE_SKELETON(impl), bus,
                                          DESKTOP_PORTAL_OBJECT_PATH, error)) {
        g_object_unref(impl);
        return FALSE;
    }
    return TRUE;
}
