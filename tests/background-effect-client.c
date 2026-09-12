#define _GNU_SOURCE
#include <assert.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <wayland-client.h>
#include "background-effect-client.h"
#include "layer-shell-client.h"

static struct wl_display *display;
static struct wl_compositor *compositor;
static struct wl_subcompositor *subcompositor;
static struct wl_shm *shm;
static struct zwlr_layer_shell_v1 *layers;
static struct ext_background_effect_manager_v1 *manager;
static uint32_t capabilities;
static void caps(void *data, struct ext_background_effect_manager_v1 *m, uint32_t flags) { capabilities = flags; }
static const struct ext_background_effect_manager_v1_listener manager_listener = {caps};
static void global(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    if (!strcmp(interface, "wl_compositor")) compositor = wl_registry_bind(registry, name, &wl_compositor_interface, 4);
    if (!strcmp(interface, "wl_subcompositor")) subcompositor = wl_registry_bind(registry, name, &wl_subcompositor_interface, 1);
    if (!strcmp(interface, "wl_shm")) shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    if (!strcmp(interface, "zwlr_layer_shell_v1")) layers = wl_registry_bind(registry, name, &zwlr_layer_shell_v1_interface, 1);
    if (!strcmp(interface, "ext_background_effect_manager_v1")) {
        manager = wl_registry_bind(registry, name, &ext_background_effect_manager_v1_interface, 1);
        ext_background_effect_manager_v1_add_listener(manager, &manager_listener, NULL);
    }
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) {}
static const struct wl_registry_listener registry_listener = {global, removed};
static void roundtrip(void) { assert(wl_display_roundtrip(display) >= 0); }
static struct wl_buffer *buffer(int width, int height, int pattern) {
    int fd = memfd_create("background-effect-test", MFD_CLOEXEC);
    assert(fd >= 0 && ftruncate(fd, width * height * 4) == 0);
    uint32_t *pixels = mmap(NULL, width * height * 4, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    assert(pixels != MAP_FAILED);
    for (int y = 0; y < height; y++) for (int x = 0; x < width; x++)
        pixels[y * width + x] = pattern ? ((x / 4 + y / 4) % 2 ? 0xffeeeeee : 0xff222222) : 0;
    struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, width * height * 4);
    struct wl_buffer *result = wl_shm_pool_create_buffer(pool, 0, width, height, width * 4, WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool); munmap(pixels, width * height * 4); close(fd);
    return result;
}
static void configure(void *data, struct zwlr_layer_surface_v1 *layer, uint32_t serial, uint32_t w, uint32_t h) {
    zwlr_layer_surface_v1_ack_configure(layer, serial);
    if (data) *(int *)data = 1;
}
static void closed(void *data, struct zwlr_layer_surface_v1 *layer) { abort(); }
static const struct zwlr_layer_surface_v1_listener layer_listener = {configure, closed};
static struct wl_surface *layer(int width, int height, int background) {
    struct wl_surface *surface = wl_compositor_create_surface(compositor);
    struct zwlr_layer_surface_v1 *role = zwlr_layer_shell_v1_get_layer_surface(layers, surface, NULL,
        background ? ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND : ZWLR_LAYER_SHELL_V1_LAYER_TOP,
        background ? "standard-blur-pattern" : "standard-blur-target");
    int configured = 0;
    zwlr_layer_surface_v1_add_listener(role, &layer_listener, &configured);
    zwlr_layer_surface_v1_set_anchor(role, ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP | ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT);
    zwlr_layer_surface_v1_set_size(role, width, height);
    if (!background) zwlr_layer_surface_v1_set_margin(role, 128, 0, 0, 128);
    zwlr_layer_surface_v1_set_exclusive_zone(role, -1);
    wl_surface_commit(surface);
    while (!configured) assert(wl_display_dispatch(display) >= 0);
    // No later configure is expected; do not retain a pointer into this stack.
    wl_proxy_set_user_data((struct wl_proxy *)role, NULL);
    wl_surface_attach(surface, buffer(width, height, background), 0, 0);
    wl_surface_damage(surface, 0, 0, width, height);
    wl_surface_commit(surface);
    return surface;
}
static void set_region(struct ext_background_effect_surface_v1 *effect, int full) {
    struct wl_region *region = wl_compositor_create_region(compositor);
    if (full) wl_region_add(region, -100, -100, 1000, 1000);
    else {
        wl_region_add(region, 16, 16, 80, 80);
        wl_region_subtract(region, 32, 32, 24, 24);
        wl_region_add(region, 144, 16, 80, 80);
    }
    ext_background_effect_surface_v1_set_blur_region(effect, region);
    // Both mutation and destruction after set must leave the copied region intact.
    wl_region_add(region, 0, 0, 320, 240);
    wl_region_destroy(region);
}
int main(int argc, char **argv) {
    assert(argc == 2);
    setvbuf(stdout, NULL, _IOLBF, 0);
    display = wl_display_connect(NULL); assert(display);
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    roundtrip(); roundtrip();
    if (!strcmp(argv[1], "absent")) { assert(!manager); puts("PASS: protocol absent outside Gnoblin"); return 0; }
    assert(manager && (capabilities & EXT_BACKGROUND_EFFECT_MANAGER_V1_CAPABILITY_BLUR));
    if (!strcmp(argv[1], "duplicate") || !strcmp(argv[1], "dead")) {
        struct wl_surface *surface = wl_compositor_create_surface(compositor);
        struct ext_background_effect_surface_v1 *effect = ext_background_effect_manager_v1_get_background_effect(manager, surface);
        if (!strcmp(argv[1], "duplicate")) ext_background_effect_manager_v1_get_background_effect(manager, surface);
        else { wl_surface_destroy(surface); ext_background_effect_surface_v1_set_blur_region(effect, NULL); }
        assert(wl_display_roundtrip(display) == -1 && wl_display_get_error(display) == EPROTO);
        const struct wl_interface *interface;
        uint32_t id;
        assert(wl_display_get_protocol_error(display, &interface, &id) == 0);
        assert(interface == (!strcmp(argv[1], "duplicate") ? &ext_background_effect_manager_v1_interface : &ext_background_effect_surface_v1_interface));
        printf("PASS: %s error\n", argv[1]); return 0;
    }
    assert(layers && shm && subcompositor);
    layer(1280, 800, 1);
    struct wl_surface *surface = layer(320, 240, 0);
    struct ext_background_effect_surface_v1 *effect = ext_background_effect_manager_v1_get_background_effect(manager, surface);
    wl_surface_commit(surface); roundtrip();
    struct wl_surface *child = NULL;
    puts("READY");
    char line[80];
    while (fgets(line, sizeof line, stdin)) {
        if (!strncmp(line, "set", 3)) set_region(effect, 0);
        else if (!strncmp(line, "full", 4)) set_region(effect, 1);
        else if (!strncmp(line, "clear", 5)) ext_background_effect_surface_v1_set_blur_region(effect, NULL);
        else if (!strncmp(line, "empty", 5)) {
            struct wl_region *empty = wl_compositor_create_region(compositor);
            ext_background_effect_surface_v1_set_blur_region(effect, empty); wl_region_destroy(empty);
        }
        else if (!strncmp(line, "commit", 6)) wl_surface_commit(surface);
        else if (!strncmp(line, "destroy", 7)) { ext_background_effect_surface_v1_destroy(effect); effect = NULL; }
        else if (!strncmp(line, "recreate", 8)) effect = ext_background_effect_manager_v1_get_background_effect(manager, surface);
        else if (!strncmp(line, "manager", 7)) { ext_background_effect_manager_v1_destroy(manager); manager = NULL; }
        else if (!strncmp(line, "scale", 5)) {
            wl_surface_set_buffer_scale(surface, 2);
            wl_surface_attach(surface, buffer(640, 480, 0), 0, 0);
            wl_surface_damage_buffer(surface, 0, 0, 640, 480);
            wl_surface_commit(surface);
        }
        else if (!strncmp(line, "child", 5)) {
            child = wl_compositor_create_surface(compositor);
            struct wl_subsurface *sub = wl_subcompositor_get_subsurface(subcompositor, child, surface);
            wl_subsurface_set_position(sub, 32, 128);
            struct ext_background_effect_surface_v1 *child_effect = ext_background_effect_manager_v1_get_background_effect(manager, child);
            set_region(child_effect, 1);
            wl_surface_attach(child, buffer(96, 80, 0), 0, 0);
            wl_surface_damage(child, 0, 0, 96, 80);
            wl_surface_commit(child);
        }
        else if (!strncmp(line, "quit", 4)) break;
        else abort();
        roundtrip(); puts("OK");
    }
    wl_display_disconnect(display);
    return 0;
}
