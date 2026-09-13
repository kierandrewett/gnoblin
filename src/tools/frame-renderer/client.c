/* SPDX-License-Identifier: MIT */
/* Language-neutral protocol transport. No GNOME/GJS dependency. */
#define _GNU_SOURCE
#include "gnoblin-window-frame-v1-client-protocol.h"
#include "paint.h"
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <libgen.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/inotify.h>
#include <sys/mman.h>
#include <unistd.h>
#include <wayland-client.h>

struct Buffer {
    struct wl_buffer* buffer;
    void* data;
    size_t size;
    struct Frame* frame;
};
struct Frame {
    struct Frame* next;
    struct gnoblin_window_frame_v1* handle;
    struct wl_surface* surface;
    struct FramePaint model;
    uint32_t serial;
    char *title, *style;
    unsigned inflight;
    int dirty, closed;
};
static struct wl_display* display;
static struct wl_compositor* compositor;
static struct wl_shm* shm;
static struct Frame* frames;
static uint32_t color = 0xff204080;
static const char* theme_path;
static int watch_fd = -1;
__attribute__((weak)) void* frame_paint_create(void) {
    return NULL;
}
__attribute__((weak)) void frame_paint_destroy(void* view) {}
__attribute__((weak)) int frame_paint_regions(const struct FramePaint* m, FrameRegion emit,
                                              void* context) {
    return 0;
}
__attribute__((weak)) int frame_paint_dispatch(void) {
    return 0;
}
__attribute__((weak)) int frame_paint_timeout(void) {
    return -1;
}
__attribute__((weak)) int frame_paint_needs_redraw(void* view) {
    return 1;
}
__attribute__((weak)) int frame_paint_poll(struct pollfd* fds, nfds_t count, int timeout) {
    return poll(fds, count, timeout);
}
static void emit_region(void* context, uint32_t action, int x, int y, int w, int h) {
    struct Frame* frame = context;
    if (x >= 0 && y >= 0 && w >= 0 && h >= 0)
        gnoblin_window_frame_v1_region(frame->handle, action, x, y, w, h);
}

static void render(struct Frame* frame);
static void free_frame(struct Frame* f) {
    free(f->title);
    free(f->style);
    frame_paint_destroy(f->model.view);
    free(f);
}
static void released(void* data, struct wl_buffer* buffer) {
    struct Buffer* b = data;
    struct Frame* f = b->frame;
    wl_buffer_destroy(buffer);
    munmap(b->data, b->size);
    free(b);
    f->inflight--;
    if (f->closed) {
        if (!f->inflight)
            free_frame(f);
    } else if (f->dirty)
        render(f);
}
static const struct wl_buffer_listener buffer_listener = {released};
static void render(struct Frame* frame) {
    frame->dirty = 1;
    if (frame->inflight >= 2)
        return;
    int w = frame->model.width, h = frame->model.height;
    if (w <= 0 || h <= 0 || w > 16384 || h > 16384 || (size_t)w * h > 32000000)
        return;
    size_t size = (size_t)w * h * 4;
    int fd = memfd_create("gnoblin-frame", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, size) < 0) {
        if (fd >= 0)
            close(fd);
        return;
    }
    struct Buffer* b = calloc(1, sizeof(*b));
    b->size = size;
    b->data = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (b->data == MAP_FAILED) {
        close(fd);
        free(b);
        return;
    }
    frame->model.title = frame->title;
    frame->model.style = frame->style;
    frame->model.color = color;
    frame_paint(b->data, &frame->model);
    struct wl_shm_pool* pool = wl_shm_create_pool(shm, fd, size);
    b->buffer = wl_shm_pool_create_buffer(pool, 0, w, h, w * 4, WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    wl_buffer_add_listener(b->buffer, &buffer_listener, b);
    b->frame = frame;
    frame->inflight++;
    frame->dirty = 0;
    gnoblin_window_frame_v1_ack_configure(frame->handle, frame->serial);
    gnoblin_window_frame_v1_clear_regions(frame->handle);
    int t = frame->model.top, r = frame->model.right, bt = frame->model.bottom,
        l = frame->model.left;
    gnoblin_window_frame_v1_region(frame->handle, 1, 0, 0, w, t);
    if (!frame_paint_regions(&frame->model, emit_region, frame))
        for (int i = 0; i < 3; i++)
            if (w - r - (i + 1) * t >= 0)
                gnoblin_window_frame_v1_region(frame->handle, 2 + i, w - r - (i + 1) * t, 0, t, t);
    gnoblin_window_frame_v1_region(frame->handle, 5, 0, 0, w, t < 4 ? t : 4);
    gnoblin_window_frame_v1_region(frame->handle, 7, w - r, t, r, h - t);
    gnoblin_window_frame_v1_region(frame->handle, 9, 0, h - bt, w, bt);
    gnoblin_window_frame_v1_region(frame->handle, 11, 0, t, l, h - t);
    wl_surface_attach(frame->surface, b->buffer, 0, 0);
    wl_surface_damage(frame->surface, 0, 0, w, h);
    wl_surface_commit(frame->surface);
}
static void configured(void* data, struct gnoblin_window_frame_v1* handle, uint32_t serial,
                       int32_t w, int32_t h, int32_t top, int32_t right, int32_t bottom,
                       int32_t left, uint32_t state, const char* title, const char* app,
                       const char* style) {
    struct Frame* f = data;
    free(f->title);
    free(f->style);
    f->title = strdup(title);
    f->style = strdup(style);
    f->serial = serial;
    f->model = (struct FramePaint){.width = w,
                                   .height = h,
                                   .top = top,
                                   .right = right,
                                   .bottom = bottom,
                                   .left = left,
                                   .state = state,
                                   .color = color,
                                   .title = f->title,
                                   .style = f->style,
                                   .view = f->model.view,
                                   .hover = f->model.hover,
                                   .pressed = f->model.pressed};
    render(f);
}
static void interaction(void* data, struct gnoblin_window_frame_v1* handle, uint32_t action,
                        uint32_t pressed) {
    struct Frame* f = data;
    if (f->model.hover == action && f->model.pressed == pressed)
        return;
    f->model.hover = action;
    f->model.pressed = pressed;
    render(f);
}
static void closed(void* data, struct gnoblin_window_frame_v1* handle) {
    struct Frame *f = data, **p = &frames;
    while (*p && *p != f)
        p = &(*p)->next;
    if (*p)
        *p = f->next;
    gnoblin_window_frame_v1_destroy(f->handle);
    wl_surface_destroy(f->surface);
    f->closed = 1;
    if (!f->inflight)
        free_frame(f);
}
static const struct gnoblin_window_frame_v1_listener frame_listener = {configured, interaction,
                                                                       closed};
static void offered(void* data, struct gnoblin_window_frame_manager_v1* manager,
                    struct gnoblin_window_frame_v1* handle) {
    struct Frame* f = calloc(1, sizeof(*f));
    f->handle = handle;
    f->model.view = frame_paint_create();
    f->surface = wl_compositor_create_surface(compositor);
    f->next = frames;
    frames = f;
    gnoblin_window_frame_v1_add_listener(handle, &frame_listener, f);
    gnoblin_window_frame_v1_attach_surface(handle, f->surface);
}
static const struct gnoblin_window_frame_manager_v1_listener manager_listener = {offered};
static int manager_found;
static void global_added(void* data, struct wl_registry* registry, uint32_t name,
                         const char* interface, uint32_t version) {
    if (!strcmp(interface, "wl_compositor"))
        compositor = wl_registry_bind(registry, name, &wl_compositor_interface, 4);
    else if (!strcmp(interface, "wl_shm"))
        shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    else if (!strcmp(interface, "gnoblin_window_frame_manager_v1")) {
        struct gnoblin_window_frame_manager_v1* manager =
            wl_registry_bind(registry, name, &gnoblin_window_frame_manager_v1_interface, 1);
        gnoblin_window_frame_manager_v1_add_listener(manager, &manager_listener, NULL);
        manager_found = 1;
    }
}
static void global_removed(void* data, struct wl_registry* registry, uint32_t name) {}
static const struct wl_registry_listener registry_listener = {global_added, global_removed};
static void load_theme(void) {
    if (!theme_path)
        return;
    FILE* file = fopen(theme_path, "r");
    if (!file)
        return;
    char line[32];
    unsigned rgb;
    char tail;
    if (fgets(line, sizeof(line), file)) {
        size_t length = strcspn(line, "\r\n");
        int valid = length == 7 && line[0] == '#';
        for (size_t i = 1; valid && i < 7; i++)
            valid = isxdigit((unsigned char)line[i]);
        if (valid && sscanf(line, "#%6x %c", &rgb, &tail) == 1)
            color = 0xff000000 | rgb;
    }
    fclose(file);
}
int main(int argc, char** argv) {
    for (int i = 1; i < argc; i++)
        if (!strncmp(argv[i], "--theme-file=", 13))
            theme_path = argv[i] + 13;
    load_theme();
    if (theme_path) {
        char* copy = strdup(theme_path);
        watch_fd = inotify_init1(IN_NONBLOCK | IN_CLOEXEC);
        if (watch_fd >= 0)
            inotify_add_watch(watch_fd, dirname(copy), IN_CLOSE_WRITE | IN_MOVED_TO | IN_DELETE);
        free(copy);
    }
    display = wl_display_connect(NULL);
    if (!display)
        return 1;
    /* GTK/toolkit display connections must not consume our privileged fd. */
    unsetenv("WAYLAND_SOCKET");
    frame_paint_init(&argc, &argv);
    struct wl_registry* registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    if (wl_display_roundtrip(display) < 0 || !compositor || !shm || !manager_found) {
        fprintf(stderr, "No private frame renderer capability on this connection\n");
        return 2;
    }
    while (1) {
        if (frame_paint_dispatch())
            for (struct Frame* f = frames; f; f = f->next)
                if (f->serial && frame_paint_needs_redraw(f->model.view))
                    render(f);
        while (wl_display_prepare_read(display) != 0)
            if (wl_display_dispatch_pending(display) < 0)
                return 1;
        int flushed = wl_display_flush(display);
        if (flushed < 0 && errno != EAGAIN) {
            wl_display_cancel_read(display);
            break;
        }
        struct pollfd fds[2] = {
            {wl_display_get_fd(display), POLLIN | (flushed < 0 ? POLLOUT : 0), 0},
            {watch_fd, POLLIN, 0}};
        int result = frame_paint_poll(fds, watch_fd >= 0 ? 2 : 1, frame_paint_timeout());
        if (result < 0) {
            wl_display_cancel_read(display);
            if (errno == EINTR)
                continue;
            break;
        }
        if (fds[0].revents & POLLIN) {
            if (wl_display_read_events(display) < 0)
                break;
        } else
            wl_display_cancel_read(display);
        if (fds[0].revents & (POLLERR | POLLHUP))
            break;
        if (wl_display_dispatch_pending(display) < 0)
            break;
        if (watch_fd >= 0 && (fds[1].revents & POLLIN)) {
            char events[4096];
            while (read(watch_fd, events, sizeof(events)) > 0) {
            }
            uint32_t old = color;
            load_theme();
            if (old != color)
                for (struct Frame* f = frames; f; f = f->next)
                    if (f->serial)
                        render(f);
        }
    }
    wl_display_disconnect(display);
    return 0;
}
