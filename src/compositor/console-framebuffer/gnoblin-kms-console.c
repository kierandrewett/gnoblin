/* SPDX-License-Identifier: GPL-2.0-or-later */

#include "config.h"

#include "backends/native/gnoblin-kms-console.h"

#include <drm_fourcc.h>
#include <errno.h>
#include <string.h>
#include <sys/mman.h>
#include <xf86drm.h>
#include <xf86drmMode.h>

#include "backends/native/meta-thread.h"
#include "backends/native/meta-backend-native.h"
#include "backends/native/meta-kms-device-private.h"
#include "backends/native/meta-kms-device.h"
#include "backends/native/meta-kms-impl-device.h"
#include "backends/native/meta-kms-private.h"
#include "backends/native/meta-kms.h"

/* A console framebuffer of 8K at four bytes per pixel is 132 MB. Anything past this is not one. */
#define MAX_CONSOLE_BYTES (256u * 1024u * 1024u)

typedef struct {
    MetaKmsDevice* device;
    uint32_t fb_id;
    int width;
    int height;
} ConsoleTarget;

struct _GnoblinKmsConsole {
    MetaKms* kms;
    GArray* targets;
};

static void console_target_clear(gpointer data) {
    ConsoleTarget* target = data;
    g_clear_object(&target->device);
}

/* One plane of linear XRGB8888 or ARGB8888: the layout the kernel's fbdev emulation uses, and the
 * only one this module writes. Any other framebuffer is not the console's, so it is left alone. */
static gboolean console_layout_is_writable(const drmModeFB2* fb) {
    if (fb->pixel_format != DRM_FORMAT_XRGB8888 && fb->pixel_format != DRM_FORMAT_ARGB8888)
        return FALSE;
    if ((fb->flags & DRM_MODE_FB_MODIFIERS) && fb->modifier != DRM_FORMAT_MOD_LINEAR)
        return FALSE;
    if (!fb->handles[0] || fb->width == 0 || fb->height == 0 || fb->pitches[0] < fb->width * 4u)
        return FALSE;
    return fb->offsets[0] + (guint64)fb->pitches[0] * fb->height <= MAX_CONSOLE_BYTES;
}

/* drmModeGetFB2 gives the caller its own handle for each buffer object. Release them all. */
static void console_close_handles(int fd, const drmModeFB2* fb) {
    for (int i = 0; i < 4; i++) {
        if (!fb->handles[i])
            continue;
        gboolean repeated = FALSE;
        for (int j = 0; j < i; j++)
            repeated = repeated || fb->handles[j] == fb->handles[i];
        if (!repeated)
            drmCloseBufferHandle(fd, fb->handles[i]);
    }
}

typedef struct {
    MetaKmsDevice* device;
    GArray* targets;
} CaptureData;

static gboolean console_has_target(GArray* targets, MetaKmsDevice* device, uint32_t fb_id) {
    for (guint i = 0; i < targets->len; i++) {
        const ConsoleTarget* target = &g_array_index(targets, ConsoleTarget, i);
        if (target->device == device && target->fb_id == fb_id)
            return TRUE;
    }
    return FALSE;
}

static gpointer capture_in_impl(MetaThreadImpl* thread_impl, gpointer user_data, GError** error) {
    CaptureData* data = user_data;
    MetaKmsImplDevice* impl_device = meta_kms_device_get_impl_device(data->device);
    int fd = meta_kms_impl_device_get_fd(impl_device);
    if (fd < 0)
        return GINT_TO_POINTER(TRUE);
    drmModeRes* resources = drmModeGetResources(fd);
    if (!resources)
        return GINT_TO_POINTER(TRUE);
    for (int i = 0; i < resources->count_crtcs; i++) {
        drmModeCrtc* crtc = drmModeGetCrtc(fd, resources->crtcs[i]);
        if (!crtc)
            continue;
        if (crtc->buffer_id && crtc->mode_valid &&
            !console_has_target(data->targets, data->device, crtc->buffer_id)) {
            drmModeFB2* fb = drmModeGetFB2(fd, crtc->buffer_id);
            if (!fb) {
                g_message("gnoblin: framebuffer %u cannot be read: %s", crtc->buffer_id,
                          g_strerror(errno));
            } else {
                if (console_layout_is_writable(fb)) {
                    ConsoleTarget target = {g_object_ref(data->device), crtc->buffer_id,
                                            (int)fb->width, (int)fb->height};
                    g_array_append_val(data->targets, target);
                } else {
                    g_message("gnoblin: framebuffer %u is not writable: format %08x, modifier "
                              "%" G_GINT64_MODIFIER "x, flags %x, handle %u, pitch %u",
                              crtc->buffer_id, fb->pixel_format, (guint64)fb->modifier, fb->flags,
                              fb->handles[0], fb->pitches[0]);
                }
                console_close_handles(fd, fb);
                drmModeFreeFB2(fb);
            }
        }
        drmModeFreeCrtc(crtc);
    }
    drmModeFreeResources(resources);
    return GINT_TO_POINTER(TRUE);
}

GnoblinKmsConsole* gnoblin_kms_console_capture(MetaBackend* backend) {
    g_return_val_if_fail(META_IS_BACKEND(backend), NULL);
    if (!META_IS_BACKEND_NATIVE(backend)) {
        g_message(
            "gnoblin: the backend is not native, so there is no console framebuffer to paint");
        return NULL;
    }
    MetaKms* kms = meta_backend_native_get_kms(META_BACKEND_NATIVE(backend));
    if (!kms) {
        g_message("gnoblin: the native backend has no KMS");
        return NULL;
    }
    GnoblinKmsConsole* console = g_new0(GnoblinKmsConsole, 1);
    console->kms = kms;
    console->targets = g_array_new(FALSE, TRUE, sizeof(ConsoleTarget));
    g_array_set_clear_func(console->targets, console_target_clear);
    for (GList* l = meta_kms_get_devices(kms); l; l = l->next) {
        CaptureData data = {l->data, console->targets};
        g_autoptr(GError) error = NULL;
        if (!meta_kms_run_impl_task_sync(kms, capture_in_impl, &data, &error))
            g_message("gnoblin: could not read the console framebuffer of %s: %s",
                      meta_kms_device_get_path(l->data), error ? error->message : "unknown error");
    }
    if (console->targets->len == 0) {
        g_message("gnoblin: no display shows a console framebuffer");
        gnoblin_kms_console_free(console);
        return NULL;
    }
    return console;
}

typedef struct {
    MetaKmsDevice* device;
    uint32_t fb_id;
    int width;
    int height;
    GBytes* pixels;
} PaintData;

static gpointer paint_in_impl(MetaThreadImpl* thread_impl, gpointer user_data, GError** error) {
    PaintData* data = user_data;
    MetaKmsImplDevice* impl_device = meta_kms_device_get_impl_device(data->device);
    int fd = meta_kms_impl_device_get_fd(impl_device);
    if (fd < 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "the device is not open");
        return NULL;
    }
    drmModeFB2* fb = drmModeGetFB2(fd, data->fb_id);
    if (!fb) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "framebuffer %u is gone: %s",
                    data->fb_id, g_strerror(errno));
        return NULL;
    }
    gpointer result = NULL;
    /* The id could have been reused for another framebuffer since it was captured. */
    if (!console_layout_is_writable(fb) || (int)fb->width != data->width ||
        (int)fb->height != data->height) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                    "framebuffer %u is no longer the console framebuffer", data->fb_id);
        goto out;
    }
    struct drm_mode_map_dumb map = {.handle = fb->handles[0]};
    if (drmIoctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &map) != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "the driver does not map framebuffer %u: %s", data->fb_id, g_strerror(errno));
        goto out;
    }
    size_t length = fb->offsets[0] + (size_t)fb->pitches[0] * fb->height;
    void* mapped = mmap(NULL, length, PROT_WRITE, MAP_SHARED, fd, map.offset);
    if (mapped == MAP_FAILED) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "could not map framebuffer %u: %s", data->fb_id, g_strerror(errno));
        goto out;
    }
    const guint8* source = g_bytes_get_data(data->pixels, NULL);
    size_t row = (size_t)data->width * 4;
    for (int y = 0; y < data->height; y++)
        memcpy((guint8*)mapped + fb->offsets[0] + (size_t)y * fb->pitches[0], source + y * row,
               row);
    munmap(mapped, length);
    g_message("gnoblin: painted the console framebuffer %u (%dx%d)", data->fb_id, data->width,
              data->height);
    result = GINT_TO_POINTER(TRUE);
out:
    console_close_handles(fd, fb);
    drmModeFreeFB2(fb);
    return result;
}

void gnoblin_kms_console_paint(GnoblinKmsConsole* console, GnoblinKmsConsoleRender render,
                               gpointer user_data) {
    g_return_if_fail(console != NULL);
    g_return_if_fail(render != NULL);
    for (guint i = 0; i < console->targets->len; i++) {
        const ConsoleTarget* target = &g_array_index(console->targets, ConsoleTarget, i);
        g_autoptr(GBytes) pixels = render(target->width, target->height, user_data);
        if (!pixels || g_bytes_get_size(pixels) != (gsize)target->width * target->height * 4)
            continue;
        PaintData data = {target->device, target->fb_id, target->width, target->height, pixels};
        g_autoptr(GError) error = NULL;
        if (!meta_kms_run_impl_task_sync(console->kms, paint_in_impl, &data, &error))
            g_message("gnoblin: could not paint the console framebuffer on %s: %s",
                      meta_kms_device_get_path(target->device),
                      error ? error->message : "unknown error");
    }
}

void gnoblin_kms_console_free(GnoblinKmsConsole* console) {
    if (!console)
        return;
    g_array_unref(console->targets);
    g_free(console);
}
