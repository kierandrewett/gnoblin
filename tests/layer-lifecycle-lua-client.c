/* Drive layer-shell lifecycle transitions for the Lua event snapshot test. */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#include <wayland-client.h>
#include "wlr-layer-shell-unstable-v1-client-protocol.h"

#define WIDTH 160
#define HEIGHT 80
#define NAMESPACE "gnoblin-lua-layer-lifecycle-e2e"

struct client {
    struct wl_compositor* compositor;
    struct wl_shm* shm;
    struct zwlr_layer_shell_v1* shell;
    int configured;
};

static void global(void* data, struct wl_registry* registry, uint32_t name, const char* interface,
                   uint32_t version) {
    struct client* c = data;
    if (!strcmp(interface, wl_compositor_interface.name))
        c->compositor =
            wl_registry_bind(registry, name, &wl_compositor_interface, version < 4 ? version : 4);
    else if (!strcmp(interface, wl_shm_interface.name))
        c->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    else if (!strcmp(interface, zwlr_layer_shell_v1_interface.name))
        c->shell = wl_registry_bind(registry, name, &zwlr_layer_shell_v1_interface,
                                    version < 4 ? version : 4);
}

static void global_remove(void* data, struct wl_registry* registry, uint32_t name) {
    (void)data;
    (void)registry;
    (void)name;
}
static const struct wl_registry_listener registry_listener = {global, global_remove};

static void configure(void* data, struct zwlr_layer_surface_v1* surface, uint32_t serial,
                      uint32_t width, uint32_t height) {
    struct client* c = data;
    (void)width;
    (void)height;
    c->configured = 1;
    zwlr_layer_surface_v1_ack_configure(surface, serial);
}
static void closed(void* data, struct zwlr_layer_surface_v1* surface) {
    (void)data;
    (void)surface;
}
static const struct zwlr_layer_surface_v1_listener layer_listener = {configure, closed};

static struct wl_buffer* make_buffer(struct client* c) {
    const int stride = WIDTH * 4, size = stride * HEIGHT;
    int fd = memfd_create("gnoblin-layer-lifecycle", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, size) < 0)
        return NULL;
    uint32_t* pixels = mmap(NULL, (size_t)size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (pixels == MAP_FAILED) {
        close(fd);
        return NULL;
    }
    for (int i = 0; i < size / 4; i++)
        pixels[i] = 0xff3050a0;
    munmap(pixels, (size_t)size);
    struct wl_shm_pool* pool = wl_shm_create_pool(c->shm, fd, size);
    struct wl_buffer* buffer =
        wl_shm_pool_create_buffer(pool, 0, WIDTH, HEIGHT, stride, WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    return buffer;
}

static int roundtrip(struct wl_display* display, const char* step) {
    if (wl_display_roundtrip(display) >= 0)
        return 0;
    fprintf(stderr, "layer lifecycle client: %s failed\n", step);
    return 1;
}

int main(void) {
    struct client c = {0};
    struct wl_display* display = wl_display_connect(NULL);
    if (!display) {
        fprintf(stderr, "cannot connect to test Wayland display\n");
        return 1;
    }
    struct wl_registry* registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, &c);
    if (roundtrip(display, "registry") || !c.compositor || !c.shm || !c.shell) {
        fprintf(stderr, "compositor, shm, or layer-shell global unavailable\n");
        return 1;
    }

    struct wl_surface* surface = wl_compositor_create_surface(c.compositor);
    struct zwlr_layer_surface_v1* layer = zwlr_layer_shell_v1_get_layer_surface(
        c.shell, surface, NULL, ZWLR_LAYER_SHELL_V1_LAYER_TOP, NAMESPACE);
    zwlr_layer_surface_v1_add_listener(layer, &layer_listener, &c);
    zwlr_layer_surface_v1_set_anchor(layer, ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP |
                                                ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT);
    zwlr_layer_surface_v1_set_size(layer, WIDTH, HEIGHT);
    wl_surface_commit(surface);
    while (!c.configured)
        if (wl_display_dispatch(display) < 0)
            return 1;
    puts("CLIENT:role-created-unmapped");
    fflush(stdout);
    usleep(400000);

    struct wl_buffer* buffer = make_buffer(&c);
    if (!buffer) {
        fprintf(stderr, "could not create shm buffer\n");
        return 1;
    }
    wl_surface_attach(surface, buffer, 0, 0);
    wl_surface_damage_buffer(surface, 0, 0, WIDTH, HEIGHT);
    wl_surface_commit(surface);
    if (roundtrip(display, "map"))
        return 1;
    puts("CLIENT:mapped");
    fflush(stdout);
    usleep(400000);

    wl_surface_attach(surface, NULL, 0, 0);
    wl_surface_commit(surface);
    if (roundtrip(display, "unmap"))
        return 1;
    puts("CLIENT:unmapped");
    fflush(stdout);
    /* Leave time for the test harness to query the live native snapshot. */
    usleep(2000000);

    zwlr_layer_surface_v1_destroy(layer);
    wl_surface_commit(surface);
    if (roundtrip(display, "remove"))
        return 1;
    puts("CLIENT:removed");
    fflush(stdout);
    wl_buffer_destroy(buffer);
    wl_surface_destroy(surface);
    zwlr_layer_shell_v1_destroy(c.shell);
    wl_shm_destroy(c.shm);
    wl_compositor_destroy(c.compositor);
    wl_registry_destroy(registry);
    wl_display_disconnect(display);
    return 0;
}
