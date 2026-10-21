/*
 * Minimal ext-session-lock-v1 client that, like Qt Wayland and other toolkit lockers, paces its
 * redraws on wl_surface.frame callbacks. It locks the session, paints every lock surface, then
 * repaints once per frame callback. After FRAMES callbacks on every surface it unlocks and exits 0.
 * If a surface receives no callback within TIMEOUT seconds it exits 1 and leaves the session locked
 * in the failsafe state (the caller must recover it).
 *
 * Build: wayland-scanner client-header/private-code for ext-session-lock-v1.xml, then
 *   cc -o session-lock-frame-client session-lock-frame-client.c ext-session-lock-v1-protocol.c
 * -lwayland-client Usage: session-lock-frame-client [FRAMES [TIMEOUT]]
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>

#include "ext-session-lock-v1-client-protocol.h"

struct output {
    struct wl_output* wl_output;
    struct wl_surface* surface;
    struct ext_session_lock_surface_v1* lock_surface;
    int width, height;
    int configured;
    int frames;
    int pending_callback;
    struct wl_buffer* buffer;
    struct wl_list link;
};

static struct wl_display* display;
static struct wl_compositor* compositor;
static struct wl_shm* shm;
static struct ext_session_lock_manager_v1* manager;
static struct ext_session_lock_v1* lock;
static struct wl_list outputs;
static int want_frames = 5;
static int locked, finished;

static struct wl_buffer* make_buffer(int width, int height, uint32_t color) {
    size_t size = (size_t)width * height * 4;
    int fd = memfd_create("lock-frame", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, size) < 0)
        return NULL;
    uint32_t* data = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (data == MAP_FAILED)
        return NULL;
    for (size_t i = 0; i < (size_t)width * height; i++)
        data[i] = color;
    munmap(data, size);
    struct wl_shm_pool* pool = wl_shm_create_pool(shm, fd, size);
    struct wl_buffer* buffer =
        wl_shm_pool_create_buffer(pool, 0, width, height, width * 4, WL_SHM_FORMAT_XRGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    return buffer;
}

static void paint(struct output* out);

static void frame_done(void* data, struct wl_callback* callback, uint32_t time) {
    struct output* out = data;
    wl_callback_destroy(callback);
    out->pending_callback = 0;
    out->frames++;
    printf("frame-callback output=%p count=%d\n", (void*)out->wl_output, out->frames);
    fflush(stdout);
    if (out->frames < want_frames)
        paint(out);
}

static const struct wl_callback_listener frame_listener = {.done = frame_done};

static void paint(struct output* out) {
    /* Alternate two colours so every repaint is a real new buffer. */
    uint32_t color = (out->frames & 1) ? 0xff336699 : 0xff993366;
    if (out->buffer)
        wl_buffer_destroy(out->buffer);
    out->buffer = make_buffer(out->width, out->height, color);
    wl_surface_attach(out->surface, out->buffer, 0, 0);
    wl_surface_damage_buffer(out->surface, 0, 0, out->width, out->height);
    struct wl_callback* callback = wl_surface_frame(out->surface);
    wl_callback_add_listener(callback, &frame_listener, out);
    out->pending_callback = 1;
    wl_surface_commit(out->surface);
}

static void lock_surface_configure(void* data, struct ext_session_lock_surface_v1* surface,
                                   uint32_t serial, uint32_t width, uint32_t height) {
    struct output* out = data;
    out->width = width;
    out->height = height;
    ext_session_lock_surface_v1_ack_configure(surface, serial);
    if (!out->configured) {
        out->configured = 1;
        printf("configure output=%p %ux%u\n", (void*)out->wl_output, width, height);
        fflush(stdout);
        paint(out);
    }
}

static const struct ext_session_lock_surface_v1_listener lock_surface_listener = {
    .configure = lock_surface_configure,
};

static void lock_locked(void* data, struct ext_session_lock_v1* l) {
    locked = 1;
    printf("locked\n");
    fflush(stdout);
}

static void lock_finished(void* data, struct ext_session_lock_v1* l) {
    finished = 1;
    printf("finished\n");
    fflush(stdout);
}

static const struct ext_session_lock_v1_listener lock_listener = {.locked = lock_locked,
                                                                  .finished = lock_finished};

static void registry_global(void* data, struct wl_registry* registry, uint32_t name,
                            const char* interface, uint32_t version) {
    if (!strcmp(interface, "wl_compositor")) {
        compositor = wl_registry_bind(registry, name, &wl_compositor_interface, 4);
    } else if (!strcmp(interface, "wl_shm")) {
        shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    } else if (!strcmp(interface, "ext_session_lock_manager_v1")) {
        manager = wl_registry_bind(registry, name, &ext_session_lock_manager_v1_interface, 1);
    } else if (!strcmp(interface, "wl_output")) {
        struct output* out = calloc(1, sizeof(*out));
        out->wl_output = wl_registry_bind(registry, name, &wl_output_interface, 2);
        wl_list_insert(&outputs, &out->link);
    }
}

static void registry_remove(void* data, struct wl_registry* registry, uint32_t name) {}

static const struct wl_registry_listener registry_listener = {
    .global = registry_global,
    .global_remove = registry_remove,
};

int main(int argc, char** argv) {
    int timeout = argc > 2 ? atoi(argv[2]) : 15;
    if (argc > 1)
        want_frames = atoi(argv[1]);
    wl_list_init(&outputs);
    display = wl_display_connect(NULL);
    if (!display) {
        fprintf(stderr, "cannot connect to the Wayland display\n");
        return 2;
    }
    struct wl_registry* registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    wl_display_roundtrip(display);
    if (!compositor || !shm || !manager) {
        fprintf(stderr, "missing wl_compositor, wl_shm or ext_session_lock_manager_v1\n");
        return 2;
    }
    lock = ext_session_lock_manager_v1_lock(manager);
    ext_session_lock_v1_add_listener(lock, &lock_listener, NULL);
    struct output* out;
    wl_list_for_each(out, &outputs, link) {
        out->surface = wl_compositor_create_surface(compositor);
        out->lock_surface =
            ext_session_lock_v1_get_lock_surface(lock, out->surface, out->wl_output);
        ext_session_lock_surface_v1_add_listener(out->lock_surface, &lock_surface_listener, out);
    }
    wl_display_roundtrip(display);

    struct timespec start;
    clock_gettime(CLOCK_MONOTONIC, &start);
    for (;;) {
        int done = locked && !wl_list_empty(&outputs);
        wl_list_for_each(out, &outputs, link) if (out->frames < want_frames) done = 0;
        if (done || finished)
            break;
        struct timespec now;
        clock_gettime(CLOCK_MONOTONIC, &now);
        if (now.tv_sec - start.tv_sec >= timeout) {
            wl_list_for_each(out, &outputs, link)
                printf("timeout output=%p frames=%d\n", (void*)out->wl_output, out->frames);
            return 1;
        }
        while (wl_display_prepare_read(display) != 0)
            wl_display_dispatch_pending(display);
        wl_display_flush(display);
        struct pollfd pfd = {.fd = wl_display_get_fd(display), .events = POLLIN};
        if (poll(&pfd, 1, 200) > 0)
            wl_display_read_events(display);
        else
            wl_display_cancel_read(display);
        wl_display_dispatch_pending(display);
    }
    if (finished)
        return 3;
    ext_session_lock_v1_unlock_and_destroy(lock);
    wl_display_roundtrip(display);
    printf("unlocked\n");
    return 0;
}
