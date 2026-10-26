/* Map one solid-colour layer surface and keep it mapped until the process is killed.
 *
 * usage: hosted-layer-client NAMESPACE LAYER ANCHOR SIZE COLOUR [OUTPUT]
 *   LAYER   background, bottom, top or overlay
 *   ANCHOR  top, bottom or all: a bar on that edge, or a surface covering the output
 *   SIZE    bar thickness in pixels (ignored for all)
 *   COLOUR  0xAARRGGBB
 *   OUTPUT  connector name of the wl_output to use, for example Meta-0 (default: compositor's
 * choice) Prints "MAPPED" once the first buffer is committed and the compositor has processed it.
 */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#include <wayland-client.h>
#include "wlr-layer-shell-unstable-v1-client-protocol.h"

#define MAX_OUTPUTS 8

struct output {
    struct wl_output* output;
    char name[64];
};

struct client {
    struct output outputs[MAX_OUTPUTS];
    int n_outputs;
    struct wl_compositor* compositor;
    struct wl_shm* shm;
    struct zwlr_layer_shell_v1* layer_shell;
    int configured;
    uint32_t width, height;
};

static void output_geometry(void* d, struct wl_output* o, int32_t x, int32_t y, int32_t pw,
                            int32_t ph, int32_t sp, const char* make, const char* model,
                            int32_t transform) {
    (void)d, (void)o, (void)x, (void)y, (void)pw, (void)ph, (void)sp, (void)make, (void)model,
        (void)transform;
}
static void output_mode(void* d, struct wl_output* o, uint32_t f, int32_t w, int32_t h, int32_t r) {
    (void)d, (void)o, (void)f, (void)w, (void)h, (void)r;
}
static void output_done(void* d, struct wl_output* o) {
    (void)d, (void)o;
}
static void output_scale(void* d, struct wl_output* o, int32_t f) {
    (void)d, (void)o, (void)f;
}
static void output_name(void* data, struct wl_output* o, const char* name) {
    struct output* out = data;

    (void)o;
    snprintf(out->name, sizeof out->name, "%s", name);
}
static void output_description(void* d, struct wl_output* o, const char* desc) {
    (void)d, (void)o, (void)desc;
}
static const struct wl_output_listener output_listener = {
    output_geometry, output_mode, output_done, output_scale, output_name, output_description};

static void registry_global(void* data, struct wl_registry* registry, uint32_t name,
                            const char* interface, uint32_t version) {
    struct client* c = data;

    if (strcmp(interface, wl_compositor_interface.name) == 0)
        c->compositor =
            wl_registry_bind(registry, name, &wl_compositor_interface, version < 4 ? version : 4);
    else if (strcmp(interface, wl_shm_interface.name) == 0)
        c->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    else if (strcmp(interface, zwlr_layer_shell_v1_interface.name) == 0)
        c->layer_shell = wl_registry_bind(registry, name, &zwlr_layer_shell_v1_interface,
                                          version < 4 ? version : 4);
    else if (strcmp(interface, wl_output_interface.name) == 0 && version >= 4 &&
             c->n_outputs < MAX_OUTPUTS) {
        struct output* o = &c->outputs[c->n_outputs++];

        o->output = wl_registry_bind(registry, name, &wl_output_interface, 4);
        wl_output_add_listener(o->output, &output_listener, o);
    }
}

static void registry_global_remove(void* data, struct wl_registry* registry, uint32_t name) {
    (void)data;
    (void)registry;
    (void)name;
}

static const struct wl_registry_listener registry_listener = {registry_global,
                                                              registry_global_remove};

static void layer_configure(void* data, struct zwlr_layer_surface_v1* surface, uint32_t serial,
                            uint32_t width, uint32_t height) {
    struct client* c = data;

    c->configured = 1;
    c->width = width;
    c->height = height;
    zwlr_layer_surface_v1_ack_configure(surface, serial);
}

static void layer_closed(void* data, struct zwlr_layer_surface_v1* surface) {
    (void)data;
    (void)surface;
    exit(3);
}

static const struct zwlr_layer_surface_v1_listener layer_listener = {layer_configure, layer_closed};

static struct wl_buffer* make_buffer(struct client* c, uint32_t colour) {
    int stride = (int)c->width * 4;
    int size = stride * (int)c->height;
    int fd = memfd_create("gnoblin-hosted-layer", MFD_CLOEXEC);

    if (fd < 0 || ftruncate(fd, size) < 0)
        return NULL;
    uint32_t* px = mmap(NULL, (size_t)size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (px == MAP_FAILED)
        return NULL;
    for (int i = 0; i < size / 4; i++)
        px[i] = colour;
    munmap(px, (size_t)size);
    struct wl_shm_pool* pool = wl_shm_create_pool(c->shm, fd, size);
    struct wl_buffer* buffer = wl_shm_pool_create_buffer(pool, 0, (int)c->width, (int)c->height,
                                                         stride, WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    return buffer;
}

int main(int argc, char** argv) {
    if (argc != 6 && argc != 7) {
        fprintf(stderr, "usage: %s NAMESPACE LAYER ANCHOR SIZE COLOUR [OUTPUT]\n", argv[0]);
        return 2;
    }
    const char* name_space = argv[1];
    uint32_t layer_value = strcmp(argv[2], "background") == 0 ? ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND
                           : strcmp(argv[2], "bottom") == 0   ? ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM
                           : strcmp(argv[2], "overlay") == 0  ? ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY
                                                              : ZWLR_LAYER_SHELL_V1_LAYER_TOP;
    uint32_t anchor = ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT | ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT;
    int all = strcmp(argv[3], "all") == 0;
    uint32_t size = (uint32_t)atoi(argv[4]);
    uint32_t colour = (uint32_t)strtoul(argv[5], NULL, 0);
    struct client c = {0};
    struct wl_display* display = wl_display_connect(NULL);

    if (!display) {
        fprintf(stderr, "cannot connect to the Wayland display\n");
        return 1;
    }
    struct wl_registry* registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, &c);
    wl_display_roundtrip(display);
    if (!c.compositor || !c.shm || !c.layer_shell) {
        fprintf(stderr, "missing compositor, shm or layer shell global\n");
        return 1;
    }
    wl_display_roundtrip(display); /* wl_output names arrive after the bind */
    struct wl_output* chosen = NULL;
    if (argc == 7) {
        for (int i = 0; i < c.n_outputs; i++)
            if (strcmp(c.outputs[i].name, argv[6]) == 0)
                chosen = c.outputs[i].output;
        if (!chosen) {
            fprintf(stderr, "no wl_output named %s\n", argv[6]);
            return 1;
        }
    }
    struct wl_surface* surface = wl_compositor_create_surface(c.compositor);
    struct zwlr_layer_surface_v1* layer = zwlr_layer_shell_v1_get_layer_surface(
        c.layer_shell, surface, chosen, layer_value, name_space);
    zwlr_layer_surface_v1_add_listener(layer, &layer_listener, &c);
    if (all) {
        anchor |= ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP | ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM;
        zwlr_layer_surface_v1_set_size(layer, 0, 0);
        zwlr_layer_surface_v1_set_exclusive_zone(layer, -1);
    } else {
        anchor |= strcmp(argv[3], "bottom") == 0 ? ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM
                                                 : ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP;
        zwlr_layer_surface_v1_set_size(layer, 0, size);
        zwlr_layer_surface_v1_set_exclusive_zone(layer, (int32_t)size);
    }
    zwlr_layer_surface_v1_set_anchor(layer, anchor);
    wl_surface_commit(surface);
    while (!c.configured)
        if (wl_display_dispatch(display) < 0)
            return 1;
    struct wl_buffer* buffer = make_buffer(&c, colour);
    uint32_t drawn_width = c.width, drawn_height = c.height;
    if (!buffer)
        return 1;
    wl_surface_attach(surface, buffer, 0, 0);
    wl_surface_damage_buffer(surface, 0, 0, (int32_t)c.width, (int32_t)c.height);
    wl_surface_commit(surface);
    if (wl_display_roundtrip(display) < 0)
        return 1;
    printf("MAPPED %ux%u\n", c.width, c.height);
    fflush(stdout);
    /* Follow later configures, such as a scale or monitor layout change, with a new buffer of the
     * size the compositor asks for. */
    while (wl_display_dispatch(display) >= 0) {
        if (c.width == drawn_width && c.height == drawn_height)
            continue;
        struct wl_buffer* resized = make_buffer(&c, colour);
        if (!resized)
            return 1;
        wl_surface_attach(surface, resized, 0, 0);
        wl_surface_damage_buffer(surface, 0, 0, (int32_t)c.width, (int32_t)c.height);
        wl_surface_commit(surface);
        wl_buffer_destroy(buffer);
        buffer = resized;
        drawn_width = c.width;
        drawn_height = c.height;
        printf("RESIZED %ux%u\n", c.width, c.height);
        fflush(stdout);
    }
    return 0;
}
