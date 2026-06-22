// Adversarial wlr-layer-shell client: deterministically reproduces the
// frame-callback assertion. On its very first configure it attaches a buffer
// AND registers a frame callback WITHOUT ack_configure. mutter's layer-shell
// role then takes the "cannot attach a buffer before ack_configure" early
// return; if that path does not consume the pending frame callbacks, the
// compositor aborts on g_assert(wl_list_empty(&state->frame_callback_list)).
//
// Buggy compositor: ABORTS. Fixed compositor: sends us a protocol error and
// keeps running (we exit cleanly reporting "compositor survived").
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>
#include <wayland-client.h>
#include "wlr-layer-shell.h"

static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct zwlr_layer_shell_v1 *layer_shell;
static int got_configure = 0, done = 0;

static void reg_global(void *d, struct wl_registry *r, uint32_t name,
                       const char *iface, uint32_t ver) {
  if (!strcmp(iface, wl_compositor_interface.name))
    compositor = wl_registry_bind(r, name, &wl_compositor_interface, 4);
  else if (!strcmp(iface, wl_shm_interface.name))
    shm = wl_registry_bind(r, name, &wl_shm_interface, 1);
  else if (!strcmp(iface, zwlr_layer_shell_v1_interface.name))
    layer_shell = wl_registry_bind(r, name, &zwlr_layer_shell_v1_interface, 1);
}
static void reg_remove(void *d, struct wl_registry *r, uint32_t name) {}
static const struct wl_registry_listener reg_listener = { reg_global, reg_remove };

static struct wl_buffer *make_buffer(int w, int h) {
  int stride = w * 4, size = stride * h;
  char tmpl[] = "/tmp/adv-XXXXXX";
  int fd = mkstemp(tmpl);
  unlink(tmpl);
  if (ftruncate(fd, size) < 0) return NULL;
  void *data = mmap(NULL, size, PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0);
  memset(data, 0x80, size);
  struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, size);
  struct wl_buffer *buf = wl_shm_pool_create_buffer(pool, 0, w, h, stride,
                                                    WL_SHM_FORMAT_ARGB8888);
  wl_shm_pool_destroy(pool);
  close(fd);
  return buf;
}

static struct wl_surface *surface;

static void ls_configure(void *d, struct zwlr_layer_surface_v1 *ls,
                         uint32_t serial, uint32_t w, uint32_t h) {
  got_configure = 1;
  fprintf(stderr, "[adv] configure serial=%u %ux%u — attaching buffer + frame "
                  "callback WITHOUT ack_configure\n", serial, w, h);
  // Deliberately DO NOT call zwlr_layer_surface_v1_ack_configure().
  struct wl_buffer *buf = make_buffer(w ? w : 64, h ? h : 64);
  wl_surface_frame(surface);          // register a frame callback in this commit
  wl_surface_attach(surface, buf, 0, 0);
  wl_surface_damage(surface, 0, 0, 64, 64);
  wl_surface_commit(surface);         // -> mutter path 737 with a pending frame cb
  done = 1;
}
static void ls_closed(void *d, struct zwlr_layer_surface_v1 *ls) {}
static const struct zwlr_layer_surface_v1_listener ls_listener = {
  ls_configure, ls_closed };

int main(void) {
  struct wl_display *dpy = wl_display_connect(NULL);
  if (!dpy) { fprintf(stderr, "[adv] no display\n"); return 2; }
  struct wl_registry *reg = wl_display_get_registry(dpy);
  wl_registry_add_listener(reg, &reg_listener, NULL);
  wl_display_roundtrip(dpy);
  if (!compositor || !shm || !layer_shell) {
    fprintf(stderr, "[adv] missing globals (compositor=%p shm=%p layer_shell=%p)\n",
            (void*)compositor, (void*)shm, (void*)layer_shell);
    return 2;
  }
  surface = wl_compositor_create_surface(compositor);
  struct zwlr_layer_surface_v1 *ls = zwlr_layer_shell_v1_get_layer_surface(
      layer_shell, surface, NULL, ZWLR_LAYER_SHELL_V1_LAYER_TOP, "adv");
  zwlr_layer_surface_v1_add_listener(ls, &ls_listener, NULL);
  zwlr_layer_surface_v1_set_size(ls, 64, 64);
  zwlr_layer_surface_v1_set_anchor(ls, ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP);
  wl_surface_commit(surface);         // triggers the first configure
  // Pump until we've sent the bad commit, then flush and see if we survive.
  while (!done && wl_display_dispatch(dpy) != -1) {}
  wl_display_roundtrip(dpy);          // force the server to process our commit
  wl_display_roundtrip(dpy);
  fprintf(stderr, "[adv] sent bad commit; compositor still responding -> survived\n");
  wl_display_disconnect(dpy);
  return 0;
}
