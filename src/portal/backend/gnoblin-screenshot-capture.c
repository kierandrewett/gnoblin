/* SPDX-License-Identifier: LGPL-2.1-or-later */
#include "gnoblin-screenshot-capture.h"

#include <errno.h>
#include <glib/gstdio.h>
#include <gio/gunixoutputstream.h>
#include <pipewire/pipewire.h>
#include <spa/param/video/format-utils.h>
#include <spa/buffer/meta.h>
#include <string.h>

#define SCREENCAST "org.gnome.Mutter.ScreenCast"
#define SESSION SCREENCAST ".Session"
#define STREAM SCREENCAST ".Stream"
#define CAPTURE_TIMEOUT_MS 10000
#define MAX_DIMENSION 16384
#define MAX_FRAME_BYTES (256u * 1024u * 1024u)

typedef struct {
    GnoblinScreenshotArea area;
    gboolean include_cursor;
    GError* error;
    GdkPixbuf* image;
    struct pw_loop* loop;
    struct pw_stream* stream;
    struct spa_video_info_raw format;
    gboolean have_format;
    gboolean connecting;
    gboolean stopping;
} Capture;

static void capture_free(gpointer data) {
    Capture* capture = data;
    g_clear_error(&capture->error);
    g_clear_object(&capture->image);
    g_free(capture);
}

static void fail(Capture* capture, const char* message) {
    if (!capture->error)
        g_set_error_literal(&capture->error, G_IO_ERROR, G_IO_ERROR_FAILED, message);
}

static gboolean valid_size(uint32_t width, uint32_t height) {
    return width > 0 && height > 0 && width <= MAX_DIMENSION && height <= MAX_DIMENSION &&
           (uint64_t)width * height * 4 <= MAX_FRAME_BYTES;
}

/* Copy while the PipeWire buffer is dequeued. The producer owns it again
 * after queue_buffer; a Pixbuf must never retain pointers into that mapping. */
static GdkPixbuf* copy_frame(const struct spa_buffer* buffer,
                             const struct spa_video_info_raw* format, GError** error) {
    const struct spa_data* data;
    const struct spa_chunk* chunk;
    struct spa_meta_header* header;
    struct spa_meta_region* crop;
    uint32_t width = format->size.width, height = format->size.height;
    uint64_t bytes;

    if (format->format != SPA_VIDEO_FORMAT_BGRx || !valid_size(width, height) ||
        buffer->n_datas != 1)
        goto invalid;
    data = &buffer->datas[0];
    chunk = data->chunk;
    if (!chunk || !data->data || (data->type != SPA_DATA_MemPtr && data->type != SPA_DATA_MemFd) ||
        chunk->stride < (int64_t)width * 4 || (chunk->flags & SPA_CHUNK_FLAG_CORRUPTED))
        goto invalid;
    bytes = (uint64_t)(height - 1) * chunk->stride + (uint64_t)width * 4;
    if (bytes > chunk->size || chunk->offset > data->maxsize ||
        bytes > data->maxsize - chunk->offset)
        goto invalid;
    header = spa_buffer_find_meta_data(buffer, SPA_META_Header, sizeof(*header));
    if (header && (header->flags & SPA_META_HEADER_FLAG_CORRUPTED))
        goto invalid;
    crop = spa_buffer_find_meta_data(buffer, SPA_META_VideoCrop, sizeof(*crop));
    if (crop && spa_meta_region_is_valid(crop) &&
        (crop->region.position.x != 0 || crop->region.position.y != 0 ||
         crop->region.size.width != width || crop->region.size.height != height))
        goto invalid;

    GdkPixbuf* image = gdk_pixbuf_new(GDK_COLORSPACE_RGB, FALSE, 8, (int)width, (int)height);
    if (!image) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE, "Cannot allocate screenshot");
        return NULL;
    }
    for (uint32_t y = 0; y < height; y++) {
        const uint8_t* source =
            (const uint8_t*)data->data + chunk->offset + (size_t)y * chunk->stride;
        uint8_t* target =
            gdk_pixbuf_get_pixels(image) + (size_t)y * gdk_pixbuf_get_rowstride(image);
        for (uint32_t x = 0; x < width; x++) {
            target[x * 3] = source[x * 4 + 2];
            target[x * 3 + 1] = source[x * 4 + 1];
            target[x * 3 + 2] = source[x * 4];
        }
    }
    return image;
invalid:
    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA, "Invalid screenshot frame");
    return NULL;
}

static void stream_process(void* data) {
    Capture* capture = data;
    struct pw_buffer* buffer = pw_stream_dequeue_buffer(capture->stream);
    if (!buffer)
        return;
    if (!capture->image && !capture->error && capture->have_format)
        capture->image = copy_frame(buffer->buffer, &capture->format, &capture->error);
    pw_stream_queue_buffer(capture->stream, buffer);
}

static void stream_state_changed(void* data, enum pw_stream_state old, enum pw_stream_state state,
                                 const char* error) {
    Capture* capture = data;
    if (!capture->stopping && (state == PW_STREAM_STATE_ERROR ||
                               (state == PW_STREAM_STATE_UNCONNECTED && !capture->connecting)))
        fail(capture, error ? error : "Screenshot stream disconnected");
}

static void stream_param_changed(void* data, uint32_t id, const struct spa_pod* param) {
    Capture* capture = data;
    uint8_t buffer[1024];
    struct spa_pod_builder builder = SPA_POD_BUILDER_INIT(buffer, sizeof buffer);
    const struct spa_pod* params[2];
    if (id != SPA_PARAM_Format || !param)
        return;
    if (spa_format_video_raw_parse(param, &capture->format) < 0 ||
        capture->format.format != SPA_VIDEO_FORMAT_BGRx ||
        !valid_size(capture->format.size.width, capture->format.size.height)) {
        fail(capture, "Unsupported screenshot format");
        return;
    }
    capture->have_format = TRUE;
    /* CPU-readable buffers only. Mutter supports BGRx without DMA-BUF or a
     * GPU import; MAP_BUFFERS maps MemFd into the consumer address space. */
    params[0] = spa_pod_builder_add_object(
        &builder, SPA_TYPE_OBJECT_ParamBuffers, SPA_PARAM_Buffers, SPA_PARAM_BUFFERS_dataType,
        SPA_POD_CHOICE_FLAGS_Int((1 << SPA_DATA_MemPtr) | (1 << SPA_DATA_MemFd)));
    params[1] = spa_pod_builder_add_object(&builder, SPA_TYPE_OBJECT_ParamMeta, SPA_PARAM_Meta,
                                           SPA_PARAM_META_type, SPA_POD_Id(SPA_META_Header),
                                           SPA_PARAM_META_size,
                                           SPA_POD_Int(sizeof(struct spa_meta_header)));
    if (pw_stream_update_params(capture->stream, params, G_N_ELEMENTS(params)) < 0)
        fail(capture, "Cannot negotiate screenshot buffers");
}

static const struct pw_stream_events stream_events = {
    PW_VERSION_STREAM_EVENTS,
    .state_changed = stream_state_changed,
    .param_changed = stream_param_changed,
    .process = stream_process,
};

static void stream_added(GDBusConnection* bus, const char* sender, const char* path,
                         const char* interface, const char* signal, GVariant* parameters,
                         gpointer data) {
    Capture* capture = data;
    uint32_t node;
    uint8_t buffer[1024];
    struct spa_pod_builder builder = SPA_POD_BUILDER_INIT(buffer, sizeof buffer);
    const struct spa_pod* params[1];
    if (capture->stream || capture->error)
        return;
    g_variant_get(parameters, "(u)", &node);
    if (node == PW_ID_ANY) {
        fail(capture, "Invalid screenshot PipeWire node");
        return;
    }
    capture->loop = pw_loop_new(NULL);
    if (!capture->loop) {
        fail(capture, "Cannot create screenshot PipeWire loop");
        return;
    }
    pw_loop_enter(capture->loop);
    capture->connecting = TRUE;
    /* This one-frame reader uses an ordinary GTask worker, not RT_PROCESS.
     * Skip the optional RTKit module for its short-lived client context. */
    capture->stream = pw_stream_new_simple(
        capture->loop, "Gnoblin portal screenshot",
        pw_properties_new(PW_KEY_MEDIA_TYPE, "Video", PW_KEY_MEDIA_CATEGORY, "Capture",
                          PW_KEY_MEDIA_ROLE, "Screen", "module.rt", "false", NULL),
        &stream_events, capture);
    if (!capture->stream) {
        fail(capture, "Cannot connect to PipeWire for screenshot");
        return;
    }
    params[0] = spa_pod_builder_add_object(
        &builder, SPA_TYPE_OBJECT_Format, SPA_PARAM_EnumFormat, SPA_FORMAT_mediaType,
        SPA_POD_Id(SPA_MEDIA_TYPE_video), SPA_FORMAT_mediaSubtype,
        SPA_POD_Id(SPA_MEDIA_SUBTYPE_raw), SPA_FORMAT_VIDEO_format,
        SPA_POD_Id(SPA_VIDEO_FORMAT_BGRx), SPA_FORMAT_VIDEO_size,
        SPA_POD_CHOICE_RANGE_Rectangle(&SPA_RECTANGLE(1, 1), &SPA_RECTANGLE(1, 1),
                                       &SPA_RECTANGLE(MAX_DIMENSION, MAX_DIMENSION)),
        SPA_FORMAT_VIDEO_framerate,
        SPA_POD_CHOICE_RANGE_Fraction(&SPA_FRACTION(0, 1), &SPA_FRACTION(0, 1),
                                      &SPA_FRACTION(240, 1)));
    if (pw_stream_connect(capture->stream, PW_DIRECTION_INPUT, node,
                          PW_STREAM_FLAG_AUTOCONNECT | PW_STREAM_FLAG_MAP_BUFFERS, params,
                          G_N_ELEMENTS(params)) < 0)
        fail(capture, "Cannot connect screenshot stream");
    capture->connecting = FALSE;
}

static void session_closed(GDBusConnection* bus, const char* sender, const char* path,
                           const char* interface, const char* signal, GVariant* parameters,
                           gpointer data) {
    fail(data, "Screenshot session closed by compositor");
}

static void discard_file(gpointer data) {
    g_unlink(data);
    g_free(data);
}

static GVariant* call(GDBusConnection* bus, const char* owner, const char* path,
                      const char* interface, const char* method, GVariant* parameters,
                      const GVariantType* reply_type, GCancellable* cancellable, GError** error) {
    return g_dbus_connection_call_sync(bus, owner, path, interface, method, parameters, reply_type,
                                       G_DBUS_CALL_FLAGS_NO_AUTO_START, CAPTURE_TIMEOUT_MS,
                                       cancellable, error);
}

static void capture_thread(GTask* task, gpointer source, gpointer task_data,
                           GCancellable* cancellable) {
    Capture* capture = task_data;
    g_autoptr(GMainContext) context = g_main_context_new();
    g_autoptr(GDBusConnection) bus = NULL;
    g_autoptr(GVariant) reply = NULL;
    g_autoptr(GError) error = NULL;
    g_autofree char *address = NULL, *owner = NULL, *session = NULL, *stream = NULL,
                    *filename = NULL;
    guint closed_id = 0, stream_id = 0;
    gint64 deadline = g_get_monotonic_time() + CAPTURE_TIMEOUT_MS * G_TIME_SPAN_MILLISECOND;
    g_main_context_push_thread_default(context);

    /* Each shot owns a bus connection. Even cancellation before CreateSession
     * returns releases the unseen session through Mutter's peer watcher. */
    address = g_dbus_address_get_for_bus_sync(G_BUS_TYPE_SESSION, cancellable, &error);
    if (!address)
        goto out;
    bus = g_dbus_connection_new_for_address_sync(address,
                                                 G_DBUS_CONNECTION_FLAGS_AUTHENTICATION_CLIENT |
                                                     G_DBUS_CONNECTION_FLAGS_MESSAGE_BUS_CONNECTION,
                                                 NULL, cancellable, &error);
    if (!bus)
        goto out;
    g_dbus_connection_set_exit_on_close(bus, FALSE);
    reply = call(bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                 "GetNameOwner", g_variant_new("(s)", SCREENCAST), G_VARIANT_TYPE("(s)"),
                 cancellable, &error);
    if (!reply)
        goto out;
    g_variant_get(reply, "(s)", &owner);
    g_clear_pointer(&reply, g_variant_unref);
    reply = call(bus, owner, "/org/gnome/Mutter/ScreenCast", SCREENCAST, "CreateSession",
                 g_variant_new("(a{sv})", NULL), G_VARIANT_TYPE("(o)"), cancellable, &error);
    if (!reply)
        goto out;
    g_variant_get(reply, "(o)", &session);
    g_clear_pointer(&reply, g_variant_unref);
    closed_id =
        g_dbus_connection_signal_subscribe(bus, owner, SESSION, "Closed", session, NULL,
                                           G_DBUS_SIGNAL_FLAGS_NONE, session_closed, capture, NULL);
    GVariantBuilder properties;
    g_variant_builder_init(&properties, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&properties, "{sv}", "cursor-mode",
                          g_variant_new_uint32(capture->include_cursor ? 1 : 0));
    reply = call(bus, owner, session, SESSION, "RecordArea",
                 g_variant_new("(iiiia{sv})", capture->area.x, capture->area.y, capture->area.width,
                               capture->area.height, &properties),
                 G_VARIANT_TYPE("(o)"), cancellable, &error);
    if (!reply)
        goto out;
    g_variant_get(reply, "(o)", &stream);
    g_clear_pointer(&reply, g_variant_unref);
    stream_id =
        g_dbus_connection_signal_subscribe(bus, owner, STREAM, "PipeWireStreamAdded", stream, NULL,
                                           G_DBUS_SIGNAL_FLAGS_NONE, stream_added, capture, NULL);
    reply =
        call(bus, owner, session, SESSION, "Start", NULL, G_VARIANT_TYPE_UNIT, cancellable, &error);
    if (!reply)
        goto out;

    while (!capture->image && !capture->error && !error) {
        while (g_main_context_iteration(context, FALSE))
            ;
        if (g_cancellable_set_error_if_cancelled(cancellable, &error))
            break;
        if (g_dbus_connection_is_closed(bus))
            fail(capture, "Screenshot bus disconnected");
        if (g_get_monotonic_time() >= deadline)
            g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_TIMED_OUT,
                                "Screenshot capture timed out");
        if (capture->loop) {
            if (pw_loop_iterate(capture->loop, 20) < 0)
                fail(capture, "Screenshot PipeWire connection failed");
        } else {
            g_usleep(1000);
        }
    }
out:
    /* Deliver any queued compositor closure before publishing the image. */
    while (g_main_context_iteration(context, FALSE))
        ;
    if (stream_id)
        g_dbus_connection_signal_unsubscribe(bus, stream_id);
    if (closed_id)
        g_dbus_connection_signal_unsubscribe(bus, closed_id);
    capture->stopping = TRUE;
    if (capture->stream) {
        pw_stream_destroy(capture->stream);
        capture->stream = NULL;
    }
    if (capture->loop) {
        pw_loop_leave(capture->loop);
        pw_loop_destroy(capture->loop);
        capture->loop = NULL;
    }
    if (session) {
        g_autoptr(GVariant) stopped = g_dbus_connection_call_sync(
            bus, owner, session, SESSION, "Stop", NULL, G_VARIANT_TYPE_UNIT,
            G_DBUS_CALL_FLAGS_NO_AUTO_START, 1000, NULL, NULL);
    }
    if (bus)
        g_dbus_connection_close_sync(bus, NULL, NULL);
    g_main_context_pop_thread_default(context);
    if (!error && capture->error)
        error = g_steal_pointer(&capture->error);
    if (!error)
        g_cancellable_set_error_if_cancelled(cancellable, &error);
    if (!error && capture->image) {
        int fd = g_file_open_tmp("gnoblin-screenshot-XXXXXX", &filename, &error);
        if (fd >= 0) {
            g_autoptr(GOutputStream) output = g_unix_output_stream_new(fd, TRUE);
            if (gdk_pixbuf_save_to_stream(capture->image, output, "png", cancellable, &error, NULL))
                g_output_stream_close(output, cancellable, &error);
        }
    }
    if (!error)
        g_cancellable_set_error_if_cancelled(cancellable, &error);
    if (!error && !filename)
        g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_FAILED, "No screenshot frame received");
    if (error) {
        if (filename)
            g_unlink(filename);
        g_task_return_error(task, g_steal_pointer(&error));
    } else {
        g_task_return_pointer(task, g_steal_pointer(&filename), discard_file);
    }
}

void gnoblin_screenshot_capture_async(const GnoblinScreenshotArea* area, gboolean include_cursor,
                                      GCancellable* cancellable, GAsyncReadyCallback callback,
                                      gpointer user_data) {
    static gsize initialized;
    g_autoptr(GTask) task = g_task_new(NULL, cancellable, callback, user_data);
    if (g_once_init_enter(&initialized)) {
        pw_init(NULL, NULL);
        g_once_init_leave(&initialized, 1);
    }
    if (!valid_size(area->width, area->height) || (int64_t)area->x + area->width > G_MAXINT ||
        (int64_t)area->y + area->height > G_MAXINT) {
        g_task_return_new_error(task, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "Invalid screenshot area");
        return;
    }
    Capture* capture = g_new0(Capture, 1);
    capture->area = *area;
    capture->include_cursor = include_cursor;
    g_task_set_task_data(task, capture, capture_free);
    g_task_run_in_thread(task, capture_thread);
}

char* gnoblin_screenshot_capture_finish(GAsyncResult* result, GError** error) {
    return g_task_propagate_pointer(G_TASK(result), error);
}
