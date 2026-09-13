#include "probe.h"
#include <gdk-pixbuf/gdk-pixbuf.h>
/* Test-only exported hooks from backends/meta-stage-private.h. The ordinary
 * Clutter after-paint signal runs after swap: reading there can capture the
 * next, empty backbuffer and cannot prove damage correctness. */
extern void* meta_stage_watch_view(void* stage, ClutterStageView* view, int phase,
                                   void (*callback)(void*, ClutterStageView*, const void*,
                                                    ClutterFrame*, void*),
                                   void* data);
extern void meta_stage_remove_watch(void* stage, void* watch);
static ClutterStage* capture_stage;
static void* capture_watch;
static char* capture_path;

static void capture_before_swap(void* stage, ClutterStageView* view, const void* clip,
                                ClutterFrame* frame, void* data) {
    if (capture_path) {
        frame_probe_save(view, capture_path);
        g_clear_pointer(&capture_path, g_free);
    }
}
void frame_probe_stop(void) {
    if (capture_watch)
        meta_stage_remove_watch(capture_stage, capture_watch);
    capture_watch = NULL;
    capture_stage = NULL;
    g_clear_pointer(&capture_path, g_free);
}
void frame_probe_arm(ClutterStage* stage, const char* path) {
    if (capture_stage && capture_stage != stage)
        frame_probe_stop();
    capture_stage = stage;
    if (!capture_watch)
        capture_watch = meta_stage_watch_view(stage, NULL, 3, capture_before_swap, NULL);
    g_free(capture_path);
    capture_path = g_strdup(path);
}
gboolean frame_probe_save(ClutterStageView* view, const char* path) {
    CoglFramebuffer* fb = clutter_stage_view_get_framebuffer(view);
    int w = cogl_framebuffer_get_width(fb), h = cogl_framebuffer_get_height(fb);
    g_autofree unsigned char* data = g_malloc_n(w * h, 4);
    if (!cogl_framebuffer_read_pixels(fb, 0, 0, w, h, COGL_PIXEL_FORMAT_RGBA_8888, data))
        return FALSE;
    GdkPixbuf* pix =
        gdk_pixbuf_new_from_data(data, GDK_COLORSPACE_RGB, TRUE, 8, w, h, w * 4, NULL, NULL);
    gboolean result = gdk_pixbuf_save(pix, path, "png", NULL, NULL);
    g_object_unref(pix);
    return result;
}
