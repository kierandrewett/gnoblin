#define _GNU_SOURCE
#include <glib.h>
#include <wayland-client.h>
#include "xdg-shell-client-protocol.h"
#include <sys/mman.h>
#include <unistd.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <stdlib.h>
#include <poll.h>
#include <errno.h>
static struct wl_display* display;
static struct wl_compositor* compositor;
static struct wl_shm* shm;
static struct xdg_wm_base* wm;
static struct wl_surface* surface;
static struct wl_buffer* buffer;
static int configured;
static int64_t previous, start, next_commit;
static int64_t now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (int64_t)t.tv_sec * 1000000 + t.tv_nsec / 1000;
}
static void frame(void* data, struct wl_callback* cb, uint32_t stamp);
static const struct wl_callback_listener frame_listener = {frame};
static void commit(void) {
    struct wl_callback* cb = wl_surface_frame(surface);
    wl_callback_add_listener(cb, &frame_listener, NULL);
    wl_surface_attach(surface, buffer, 0, 0);
    wl_surface_damage_buffer(surface, 0, 0, 320, 240);
    wl_surface_commit(surface);
}
static void frame(void* data, struct wl_callback* cb, uint32_t stamp) {
    int64_t t = now();
    wl_callback_destroy(cb);
    printf("FRAME %lld %lld %u\n", (long long)(t - start), (long long)(previous ? t - previous : 0),
           stamp);
    fflush(stdout);
    previous = t;
    next_commit = t + 16667;
}
static void ping(void* data, struct xdg_wm_base* base, uint32_t serial) {
    xdg_wm_base_pong(base, serial);
}
static const struct xdg_wm_base_listener wm_listener = {ping};
static void config(void* data, struct xdg_surface* xdg, uint32_t serial) {
    xdg_surface_ack_configure(xdg, serial);
    if (!configured) {
        configured = 1;
        commit();
    }
}
static const struct xdg_surface_listener surface_listener = {config};
static void top_config(void* d, struct xdg_toplevel* t, int32_t w, int32_t h, struct wl_array* s) {}
static void closed(void* d, struct xdg_toplevel* t) {
    exit(0);
}
static void bounds(void* d, struct xdg_toplevel* t, int32_t w, int32_t h) {}
static void caps(void* d, struct xdg_toplevel* t, struct wl_array* c) {}
static const struct xdg_toplevel_listener top_listener = {top_config, closed, bounds, caps};
static void keymap(void* d, struct wl_keyboard* k, uint32_t f, int fd, uint32_t size) {
    close(fd);
}
static void enter(void* d, struct wl_keyboard* k, uint32_t serial, struct wl_surface* s,
                  struct wl_array* keys) {}
static void leave(void* d, struct wl_keyboard* k, uint32_t serial, struct wl_surface* s) {}
static void key(void* d, struct wl_keyboard* k, uint32_t serial, uint32_t stamp, uint32_t code,
                uint32_t state) {
    printf("KEY %lld %u %u\n", (long long)now(), code, state);
    fflush(stdout);
}
static void modifiers(void* d, struct wl_keyboard* k, uint32_t serial, uint32_t depressed,
                      uint32_t latched, uint32_t locked, uint32_t group) {}
static void repeat(void* d, struct wl_keyboard* k, int32_t rate, int32_t delay) {}
static const struct wl_keyboard_listener keyboard_listener = {keymap, enter,     leave,
                                                              key,    modifiers, repeat};
static void seat_caps(void* d, struct wl_seat* seat, uint32_t caps) {
    if (caps & WL_SEAT_CAPABILITY_KEYBOARD) {
        struct wl_keyboard* k = wl_seat_get_keyboard(seat);
        wl_keyboard_add_listener(k, &keyboard_listener, NULL);
    }
}
static void seat_name(void* d, struct wl_seat* s, const char* n) {}
static const struct wl_seat_listener seat_listener = {seat_caps, seat_name};
static void global(void* d, struct wl_registry* r, uint32_t n, const char* i, uint32_t v) {
    if (!strcmp(i, "wl_compositor"))
        compositor = wl_registry_bind(r, n, &wl_compositor_interface, 4);
    else if (!strcmp(i, "wl_shm"))
        shm = wl_registry_bind(r, n, &wl_shm_interface, 1);
    else if (!strcmp(i, "wl_seat")) {
        struct wl_seat* seat = wl_registry_bind(r, n, &wl_seat_interface, MIN(v, 5));
        wl_seat_add_listener(seat, &seat_listener, NULL);
    } else if (!strcmp(i, "xdg_wm_base"))
        wm = wl_registry_bind(r, n, &xdg_wm_base_interface, MIN(v, 3));
}
static void removed(void* d, struct wl_registry* r, uint32_t n) {}
static const struct wl_registry_listener registry_listener = {global, removed};
int main(void) {
    start = now();
    printf("START %lld\n", (long long)start);
    fflush(stdout);
    display = wl_display_connect(NULL);
    if (!display)
        return 1;
    struct wl_registry* r = wl_display_get_registry(display);
    wl_registry_add_listener(r, &registry_listener, NULL);
    wl_display_roundtrip(display);
    if (!compositor || !shm || !wm)
        return 2;
    xdg_wm_base_add_listener(wm, &wm_listener, NULL);
    int fd = memfd_create("frame-pressure-probe", MFD_CLOEXEC), size = 320 * 240 * 4;
    ftruncate(fd, size);
    uint32_t* pixels = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    for (int i = 0; i < 320 * 240; i++)
        pixels[i] = 0xff40b080;
    struct wl_shm_pool* pool = wl_shm_create_pool(shm, fd, size);
    buffer = wl_shm_pool_create_buffer(pool, 0, 320, 240, 320 * 4, WL_SHM_FORMAT_XRGB8888);
    wl_shm_pool_destroy(pool);
    munmap(pixels, size);
    close(fd);
    surface = wl_compositor_create_surface(compositor);
    struct xdg_surface* xdg = xdg_wm_base_get_xdg_surface(wm, surface);
    xdg_surface_add_listener(xdg, &surface_listener, NULL);
    struct xdg_toplevel* top = xdg_surface_get_toplevel(xdg);
    xdg_toplevel_add_listener(top, &top_listener, NULL);
    xdg_toplevel_set_title(top, "Frame pressure probe");
    xdg_toplevel_set_app_id(top, "org.gnoblin.PressureProbe");
    wl_surface_commit(surface);
    while (1) {
        if (wl_display_dispatch_pending(display) < 0)
            break;
        if (next_commit && now() >= next_commit) {
            next_commit = 0;
            commit();
        }
        while (wl_display_prepare_read(display) != 0)
            if (wl_display_dispatch_pending(display) < 0)
                return 1;
        if (wl_display_flush(display) < 0 && errno != EAGAIN) {
            wl_display_cancel_read(display);
            break;
        }
        int timeout = next_commit ? (int)((next_commit - now() + 999) / 1000) : -1;
        if (timeout < 0 && next_commit)
            timeout = 0;
        struct pollfd pfd = {.fd = wl_display_get_fd(display), .events = POLLIN};
        int result = poll(&pfd, 1, timeout);
        if (result > 0 && (pfd.revents & POLLIN)) {
            if (wl_display_read_events(display) < 0)
                break;
        } else
            wl_display_cancel_read(display);
        if (result < 0 && errno != EINTR)
            break;
    }
    return 0;
}
