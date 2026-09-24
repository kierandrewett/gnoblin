/* A deliberately small real zwlr-layer-shell client kept visible during app runs. */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#include <wayland-client.h>
#include "wlr-layer-shell-unstable-v1-client-protocol.h"

#define PANEL_HEIGHT 28

struct panel {
    struct wl_compositor* compositor;
    struct wl_shm* shm;
    struct zwlr_layer_shell_v1* shell;
    struct wl_surface* surface;
    struct zwlr_layer_surface_v1* layer;
    int ready;
};

static void buffer_release(void* data, struct wl_buffer* buffer) {
    (void)data;
    wl_buffer_destroy(buffer);
}

static const struct wl_buffer_listener buffer_listener = {.release = buffer_release};

static struct wl_buffer* create_buffer(struct panel* panel, uint32_t width, uint32_t height) {
    const int stride = (int)width * 4;
    const int size = stride * (int)height;
    const int fd = memfd_create("gnoblin-test-panel", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, size) < 0) {
        if (fd >= 0)
            close(fd);
        return NULL;
    }

    uint32_t* pixels = mmap(NULL, (size_t)size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (pixels == MAP_FAILED) {
        close(fd);
        return NULL;
    }
    for (int i = 0; i < size / 4; i++)
        pixels[i] = 0xff26364a;
    munmap(pixels, (size_t)size);

    struct wl_shm_pool* pool = wl_shm_create_pool(panel->shm, fd, size);
    struct wl_buffer* buffer = wl_shm_pool_create_buffer(
        pool, 0, (int)width, (int)height, stride, WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    if (buffer)
        wl_buffer_add_listener(buffer, &buffer_listener, NULL);
    return buffer;
}

static void layer_configure(void* data, struct zwlr_layer_surface_v1* layer,
                            uint32_t serial, uint32_t width, uint32_t height) {
    struct panel* panel = data;
    zwlr_layer_surface_v1_ack_configure(layer, serial);
    if (!width)
        width = 1280;
    if (!height)
        height = PANEL_HEIGHT;
    struct wl_buffer* buffer = create_buffer(panel, width, height);
    if (!buffer) {
        fprintf(stderr, "test-panel: cannot allocate %ux%u buffer\n", width, height);
        exit(1);
    }
    wl_surface_attach(panel->surface, buffer, 0, 0);
    wl_surface_damage(panel->surface, 0, 0, (int32_t)width, (int32_t)height);
    wl_surface_commit(panel->surface);
    if (!panel->ready) {
        panel->ready = 1;
        puts("GNOBLIN_TEST_SHELL_READY");
        fflush(stdout);
    }
}

static void layer_closed(void* data, struct zwlr_layer_surface_v1* layer) {
    (void)data;
    (void)layer;
    fprintf(stderr, "test-panel: compositor closed the layer surface\n");
    exit(1);
}

static const struct zwlr_layer_surface_v1_listener layer_listener = {
    .configure = layer_configure,
    .closed = layer_closed,
};

static void registry_global(void* data, struct wl_registry* registry, uint32_t name,
                            const char* interface, uint32_t version) {
    struct panel* panel = data;
    if (!strcmp(interface, wl_compositor_interface.name))
        panel->compositor = wl_registry_bind(registry, name, &wl_compositor_interface,
                                             version < 4 ? version : 4);
    else if (!strcmp(interface, wl_shm_interface.name))
        panel->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    else if (!strcmp(interface, zwlr_layer_shell_v1_interface.name))
        panel->shell = wl_registry_bind(registry, name, &zwlr_layer_shell_v1_interface,
                                        version < 4 ? version : 4);
}

static void registry_global_remove(void* data, struct wl_registry* registry, uint32_t name) {
    (void)data;
    (void)registry;
    (void)name;
}

static const struct wl_registry_listener registry_listener = {
    .global = registry_global,
    .global_remove = registry_global_remove,
};

int main(void) {
    struct panel panel = {0};
    struct wl_display* display = wl_display_connect(NULL);
    if (!display) {
        fprintf(stderr, "test-panel: cannot connect to Wayland display\n");
        return 1;
    }

    struct wl_registry* registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, &panel);
    if (wl_display_roundtrip(display) < 0 || !panel.compositor || !panel.shm || !panel.shell) {
        fprintf(stderr, "test-panel: compositor, shm, or layer-shell global unavailable\n");
        return 1;
    }

    panel.surface = wl_compositor_create_surface(panel.compositor);
    panel.layer = zwlr_layer_shell_v1_get_layer_surface(
        panel.shell, panel.surface, NULL, ZWLR_LAYER_SHELL_V1_LAYER_TOP, "gnoblin-e2e-panel");
    zwlr_layer_surface_v1_add_listener(panel.layer, &layer_listener, &panel);
    zwlr_layer_surface_v1_set_size(panel.layer, 0, PANEL_HEIGHT);
    zwlr_layer_surface_v1_set_anchor(panel.layer,
        ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP | ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT |
        ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
    zwlr_layer_surface_v1_set_exclusive_zone(panel.layer, PANEL_HEIGHT);
    zwlr_layer_surface_v1_set_keyboard_interactivity(
        panel.layer, ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_NONE);
    wl_surface_commit(panel.surface);

    while (wl_display_dispatch(display) >= 0) {}
    wl_display_disconnect(display);
    return 0;
}
