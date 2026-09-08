/* Exercise layer size requests separately from resized buffer commits. */
#define _GNU_SOURCE
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#include <wayland-client.h>
#include "wlr-layer-shell-unstable-v1-client-protocol.h"

#define BAR_WIDTH 1280
#define BAR_HEIGHT 32

static struct timespec t_start;

static uint64_t elapsed_us(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (uint64_t)(now.tv_sec - t_start.tv_sec) * 1000000 +
           (uint64_t)(now.tv_nsec - t_start.tv_nsec) / 1000;
}

struct probe {
    struct wl_compositor* compositor;
    struct wl_shm* shm;
    struct zwlr_layer_shell_v1* layer_shell;
    int configured;
    int framed;
    uint32_t width, height;
    uint64_t t_configure, t_frame;
};

static void registry_global(void* data, struct wl_registry* registry, uint32_t name,
                            const char* interface, uint32_t version) {
    struct probe* p = data;

    if (strcmp(interface, wl_compositor_interface.name) == 0)
        p->compositor =
            wl_registry_bind(registry, name, &wl_compositor_interface, version < 4 ? version : 4);
    else if (strcmp(interface, wl_shm_interface.name) == 0)
        p->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    else if (strcmp(interface, zwlr_layer_shell_v1_interface.name) == 0)
        p->layer_shell = wl_registry_bind(registry, name, &zwlr_layer_shell_v1_interface,
                                          version < 4 ? version : 4);
}

static void registry_global_remove(void* data, struct wl_registry* registry, uint32_t name) {
    (void)data;
    (void)registry;
    (void)name;
}

static const struct wl_registry_listener registry_listener = {
    registry_global,
    registry_global_remove,
};

static void layer_configure(void* data, struct zwlr_layer_surface_v1* surface, uint32_t serial,
                            uint32_t width, uint32_t height) {
    struct probe* p = data;

    /* First configure is the compositor accepting the surface; everything before
     * this is negotiation, everything after is our own drawing. */
    if (!p->configured)
        p->t_configure = elapsed_us();
    p->configured = 1;
    p->width = width ? width : BAR_WIDTH;
    p->height = height ? height : BAR_HEIGHT;
    zwlr_layer_surface_v1_ack_configure(surface, serial);
}

static void layer_closed(void* data, struct zwlr_layer_surface_v1* surface) {
    (void)data;
    (void)surface;
}

static const struct zwlr_layer_surface_v1_listener layer_listener = {
    layer_configure,
    layer_closed,
};

static void frame_done(void* data, struct wl_callback* cb, uint32_t time) {
    struct probe* p = data;
    (void)time;

    if (!p->framed)
        p->t_frame = elapsed_us();
    p->framed = 1;
    wl_callback_destroy(cb);
}

static const struct wl_callback_listener frame_listener = {frame_done};

/* Anonymous shm buffer. memfd_create keeps it off the filesystem entirely. */
static struct wl_buffer* make_buffer(struct probe* p, uint32_t w, uint32_t h) {
    int stride = (int)w * 4;
    int size = stride * (int)h;
    int fd = memfd_create("gnoblin-latency", MFD_CLOEXEC);

    if (fd < 0)
        return NULL;
    if (ftruncate(fd, size) < 0) {
        close(fd);
        return NULL;
    }

    uint32_t* px = mmap(NULL, (size_t)size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (px == MAP_FAILED) {
        close(fd);
        return NULL;
    }
    for (int i = 0; i < size / 4; i++)
        px[i] = 0xffff00ff; /* opaque dark, like a bar */
    munmap(px, (size_t)size);

    struct wl_shm_pool* pool = wl_shm_create_pool(p->shm, fd, size);
    struct wl_buffer* buf =
        wl_shm_pool_create_buffer(pool, 0, (int)w, (int)h, stride, WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    return buf;
}

int main(int argc, char** argv) {
    int top = argc > 1 && strcmp(argv[1], "top") == 0;
    int right = argc > 1 && strcmp(argv[1], "right") == 0;
    struct probe p = {0};
    struct wl_display* d = wl_display_connect(NULL);
    struct wl_registry* r = wl_display_get_registry(d);
    wl_registry_add_listener(r, &registry_listener, &p);
    wl_display_roundtrip(d);
    struct wl_surface* s = wl_compositor_create_surface(p.compositor);
    struct zwlr_layer_surface_v1* l = zwlr_layer_shell_v1_get_layer_surface(
        p.layer_shell, s, NULL, ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY, "tooltip-handshake");
    zwlr_layer_surface_v1_add_listener(l, &layer_listener, &p);
    zwlr_layer_surface_v1_set_anchor(
        l, (top || right ? ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP : ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM) |
               (right ? ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT : ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT));
    zwlr_layer_surface_v1_set_margin(l, top || right ? 100 : 0, right ? 100 : 0,
                                     top || right ? 0 : 100, right ? 0 : 100);
    zwlr_layer_surface_v1_set_size(l, 200, 32);
    wl_surface_commit(s);
    while (!p.configured)
        wl_display_dispatch(d);
    struct wl_buffer* b = make_buffer(&p, 200, 32);
    wl_surface_attach(s, b, 0, 0);
    wl_surface_damage_buffer(s, 0, 0, 200, 32);
    wl_surface_commit(s);
    wl_display_roundtrip(d);
    puts("INITIAL");
    fflush(stdout);
    getchar();
    if (top)
        zwlr_layer_surface_v1_set_margin(l, 100, 0, 0, 50);
    zwlr_layer_surface_v1_set_size(l, 300, 100);
    wl_surface_commit(s);
    wl_display_roundtrip(d);
    puts("PENDING");
    fflush(stdout);
    getchar();
    b = make_buffer(&p, 300, 100);
    wl_surface_attach(s, b, 0, 0);
    wl_surface_damage_buffer(s, 0, 0, 300, 100);
    wl_surface_commit(s);
    wl_display_roundtrip(d);
    puts("RESIZED");
    fflush(stdout);
    getchar();
    wl_display_disconnect(d);
    return 0;
}
