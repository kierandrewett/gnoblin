/* Hide and re-show one wl_surface as a layer surface, the way GTK does. */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#include <wayland-client.h>
#include "wlr-layer-shell-unstable-v1-client-protocol.h"

#define WIDTH 200
#define HEIGHT 100

struct probe {
    struct wl_compositor* compositor;
    struct wl_shm* shm;
    struct zwlr_layer_shell_v1* layer_shell;
    int configured;
    uint32_t width, height;
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

    p->configured = 1;
    p->width = width;
    p->height = height;
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

static struct wl_buffer* make_buffer(struct probe* p) {
    int stride = WIDTH * 4;
    int size = stride * HEIGHT;
    int fd = memfd_create("gnoblin-layer-remap", MFD_CLOEXEC);

    if (fd < 0 || ftruncate(fd, size) < 0) {
        if (fd >= 0)
            close(fd);
        return NULL;
    }

    uint32_t* px = mmap(NULL, (size_t)size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (px == MAP_FAILED) {
        close(fd);
        return NULL;
    }
    for (int i = 0; i < size / 4; i++)
        px[i] = 0xff3050a0;
    munmap(px, (size_t)size);

    struct wl_shm_pool* pool = wl_shm_create_pool(p->shm, fd, size);
    struct wl_buffer* buffer =
        wl_shm_pool_create_buffer(pool, 0, WIDTH, HEIGHT, stride, WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    return buffer;
}

static int fail(struct wl_display* display, int round, const char* step) {
    const struct wl_interface* interface = NULL;
    uint32_t id = 0;
    int code = wl_display_get_protocol_error(display, &interface, &id);

    fprintf(stderr, "round %d: %s failed: protocol error %d on %s@%u\n", round, step, code,
            interface ? interface->name : "?", id);
    return 1;
}

/* Usage: layer-shell-remap-client ROUNDS SETTLE
 * SETTLE 0 re-creates the layer surface straight after the null commit, as a
 * fast show/hide toggle does; 1 waits for the compositor in between. */
int main(int argc, char** argv) {
    int rounds = argc > 1 ? atoi(argv[1]) : 3;
    int settle = argc > 2 ? atoi(argv[2]) : 0;
    struct probe p = {0};
    struct wl_display* display = wl_display_connect(NULL);

    if (!display)
        return 1;
    struct wl_registry* registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, &p);
    wl_display_roundtrip(display);
    if (!p.compositor || !p.shm || !p.layer_shell)
        return 1;

    struct wl_surface* surface = wl_compositor_create_surface(p.compositor);
    for (int round = 1; round <= rounds; round++) {
        p.configured = 0;
        struct zwlr_layer_surface_v1* layer = zwlr_layer_shell_v1_get_layer_surface(
            p.layer_shell, surface, NULL, ZWLR_LAYER_SHELL_V1_LAYER_TOP, "remap-test");
        zwlr_layer_surface_v1_add_listener(layer, &layer_listener, &p);
        zwlr_layer_surface_v1_set_anchor(layer, ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP |
                                                    ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
        zwlr_layer_surface_v1_set_size(layer, WIDTH, HEIGHT);
        wl_surface_commit(surface);
        /* Identical requests every round: a surface that kept its old state
         * would never be sent an initial configure, and this would wait. */
        while (!p.configured)
            if (wl_display_dispatch(display) < 0)
                return fail(display, round, "configure");
        if (p.width != WIDTH || p.height != HEIGHT) {
            fprintf(stderr, "round %d: configured %ux%u, wanted %ux%u\n", round, p.width, p.height,
                    WIDTH, HEIGHT);
            return 1;
        }

        struct wl_buffer* buffer = make_buffer(&p);
        wl_surface_attach(surface, buffer, 0, 0);
        wl_surface_damage_buffer(surface, 0, 0, WIDTH, HEIGHT);
        wl_surface_commit(surface);
        if (wl_display_roundtrip(display) < 0)
            return fail(display, round, "map");
        printf("MAPPED %d\n", round);
        fflush(stdout);

        /* GTK's hide: the role object first, then an empty commit. */
        zwlr_layer_surface_v1_destroy(layer);
        wl_surface_attach(surface, NULL, 0, 0);
        wl_surface_commit(surface);
        if (settle && wl_display_roundtrip(display) < 0)
            return fail(display, round, "unmap");
        wl_buffer_destroy(buffer);
    }
    if (wl_display_roundtrip(display) < 0)
        return fail(display, rounds, "final roundtrip");
    wl_display_disconnect(display);
    return 0;
}
