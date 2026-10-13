/* SPDX-License-Identifier: LGPL-2.1-or-later */
#pragma once

#include <gio/gio.h>
#include <gdk-pixbuf/gdk-pixbuf.h>

typedef struct {
    int x, y, width, height;
} GnoblinScreenshotArea;

/* Backend-private adapter for the pinned Mutter ScreenCast API. The caller
 * authorizes the request and supplies compositor (logical) coordinates. */
void gnoblin_screenshot_capture_async(const GnoblinScreenshotArea* area, gboolean include_cursor,
                                      GCancellable* cancellable, GAsyncReadyCallback callback,
                                      gpointer user_data);
/* Returns an owned filename. On rejection/cancellation the caller must unlink
 * it; on success the portal frontend needs it to remain available. */
char* gnoblin_screenshot_capture_finish(GAsyncResult* result, GError** error);
