/* layer-shell-latency-client -- measure how long a layer-shell bar takes to
 * put its first pixel on screen.
 *
 * gnoblin draws no chrome of its own: every bar, dock and panel is a
 * wlr-layer-shell client. So the latency that decides whether the desktop
 * feels snappy is not boot time, it is this -- process start to first frame
 * presented. Nothing measured it before.
 *
 * Reports four cumulative marks, all CLOCK_MONOTONIC from process start:
 *
 *   connect    wl_display_connect() returned
 *   globals    registry roundtrip done (compositor, shm, layer_shell bound)
 *   configure  compositor sent the first layer_surface configure
 *   frame      our first frame callback fired (first pixel is up)
 *
 * Machine-readable last line: "LAYER_SHELL_LATENCY <connect> <globals>
 * <configure> <frame>" in integer microseconds, for scripts/test-layer-latency.sh.
 *
 * Deliberately allocates its own shm buffer rather than using a toolkit: the
 * point is to measure the compositor path, not GTK or EGL startup. A real
 * client pays those on top (see TODO.md "Performance" -- an EGL context alone
 * was ~33 ms on the old Slint clients).
 */
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

static uint64_t
elapsed_us (void)
{
  struct timespec now;
  clock_gettime (CLOCK_MONOTONIC, &now);
  return (uint64_t) (now.tv_sec - t_start.tv_sec) * 1000000
         + (uint64_t) (now.tv_nsec - t_start.tv_nsec) / 1000;
}

struct probe
{
  struct wl_compositor *compositor;
  struct wl_shm *shm;
  struct zwlr_layer_shell_v1 *layer_shell;
  int configured;
  int framed;
  uint32_t width, height;
  uint64_t t_configure, t_frame;
};

static void
registry_global (void *data, struct wl_registry *registry, uint32_t name,
                 const char *interface, uint32_t version)
{
  struct probe *p = data;

  if (strcmp (interface, wl_compositor_interface.name) == 0)
    p->compositor = wl_registry_bind (registry, name, &wl_compositor_interface,
                                      version < 4 ? version : 4);
  else if (strcmp (interface, wl_shm_interface.name) == 0)
    p->shm = wl_registry_bind (registry, name, &wl_shm_interface, 1);
  else if (strcmp (interface, zwlr_layer_shell_v1_interface.name) == 0)
    p->layer_shell = wl_registry_bind (registry, name,
                                       &zwlr_layer_shell_v1_interface,
                                       version < 4 ? version : 4);
}

static void
registry_global_remove (void *data, struct wl_registry *registry, uint32_t name)
{
  (void) data; (void) registry; (void) name;
}

static const struct wl_registry_listener registry_listener = {
  registry_global, registry_global_remove,
};

static void
layer_configure (void *data, struct zwlr_layer_surface_v1 *surface,
                 uint32_t serial, uint32_t width, uint32_t height)
{
  struct probe *p = data;

  /* First configure is the compositor accepting the surface; everything before
   * this is negotiation, everything after is our own drawing. */
  if (!p->configured)
    p->t_configure = elapsed_us ();
  p->configured = 1;
  p->width = width ? width : BAR_WIDTH;
  p->height = height ? height : BAR_HEIGHT;
  zwlr_layer_surface_v1_ack_configure (surface, serial);
}

static void
layer_closed (void *data, struct zwlr_layer_surface_v1 *surface)
{
  (void) data; (void) surface;
}

static const struct zwlr_layer_surface_v1_listener layer_listener = {
  layer_configure, layer_closed,
};

static void
frame_done (void *data, struct wl_callback *cb, uint32_t time)
{
  struct probe *p = data;
  (void) time;

  if (!p->framed)
    p->t_frame = elapsed_us ();
  p->framed = 1;
  wl_callback_destroy (cb);
}

static const struct wl_callback_listener frame_listener = { frame_done };

/* Anonymous shm buffer. memfd_create keeps it off the filesystem entirely. */
static struct wl_buffer *
make_buffer (struct probe *p, uint32_t w, uint32_t h)
{
  int stride = (int) w * 4;
  int size = stride * (int) h;
  int fd = memfd_create ("gnoblin-latency", MFD_CLOEXEC);

  if (fd < 0)
    return NULL;
  if (ftruncate (fd, size) < 0)
    {
      close (fd);
      return NULL;
    }

  uint32_t *px = mmap (NULL, (size_t) size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  if (px == MAP_FAILED)
    {
      close (fd);
      return NULL;
    }
  for (int i = 0; i < size / 4; i++)
    px[i] = 0xff202020;   /* opaque dark, like a bar */
  munmap (px, (size_t) size);

  struct wl_shm_pool *pool = wl_shm_create_pool (p->shm, fd, size);
  struct wl_buffer *buf = wl_shm_pool_create_buffer (pool, 0, (int) w, (int) h,
                                                     stride, WL_SHM_FORMAT_ARGB8888);
  wl_shm_pool_destroy (pool);
  close (fd);
  return buf;
}

int
main (void)
{
  struct probe p = { 0 };
  uint64_t t_connect, t_globals;

  clock_gettime (CLOCK_MONOTONIC, &t_start);

  struct wl_display *display = wl_display_connect (NULL);
  if (!display)
    {
      fprintf (stderr, "layer-latency: cannot connect to WAYLAND_DISPLAY\n");
      return 1;
    }
  t_connect = elapsed_us ();

  struct wl_registry *registry = wl_display_get_registry (display);
  wl_registry_add_listener (registry, &registry_listener, &p);
  wl_display_roundtrip (display);
  t_globals = elapsed_us ();

  if (!p.compositor || !p.shm || !p.layer_shell)
    {
      fprintf (stderr, "layer-latency: missing globals (compositor=%p shm=%p layer_shell=%p)\n",
               (void *) p.compositor, (void *) p.shm, (void *) p.layer_shell);
      return 1;
    }

  struct wl_surface *surface = wl_compositor_create_surface (p.compositor);
  struct zwlr_layer_surface_v1 *layer =
    zwlr_layer_shell_v1_get_layer_surface (p.layer_shell, surface, NULL,
                                           ZWLR_LAYER_SHELL_V1_LAYER_TOP,
                                           "gnoblin-latency");
  zwlr_layer_surface_v1_set_size (layer, BAR_WIDTH, BAR_HEIGHT);
  zwlr_layer_surface_v1_set_anchor (layer,
                                    ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP
                                    | ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT
                                    | ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
  zwlr_layer_surface_v1_add_listener (layer, &layer_listener, &p);
  wl_surface_commit (surface);

  while (!p.configured && wl_display_dispatch (display) != -1)
    ;
  if (!p.configured)
    {
      fprintf (stderr, "layer-latency: never configured\n");
      return 1;
    }

  struct wl_buffer *buf = make_buffer (&p, p.width, p.height);
  if (!buf)
    {
      fprintf (stderr, "layer-latency: buffer allocation failed: %s\n", strerror (errno));
      return 1;
    }

  struct wl_callback *cb = wl_surface_frame (surface);
  wl_callback_add_listener (cb, &frame_listener, &p);
  wl_surface_attach (surface, buf, 0, 0);
  wl_surface_damage_buffer (surface, 0, 0, (int) p.width, (int) p.height);
  wl_surface_commit (surface);

  while (!p.framed && wl_display_dispatch (display) != -1)
    ;
  if (!p.framed)
    {
      fprintf (stderr, "layer-latency: no frame callback\n");
      return 1;
    }

  printf ("   connect   %6.2f ms\n", t_connect / 1000.0);
  printf ("   globals   %6.2f ms\n", t_globals / 1000.0);
  printf ("   configure %6.2f ms\n", p.t_configure / 1000.0);
  printf ("   frame     %6.2f ms  <- first pixel on screen\n", p.t_frame / 1000.0);
  printf ("LAYER_SHELL_LATENCY %lu %lu %lu %lu\n",
          (unsigned long) t_connect, (unsigned long) t_globals,
          (unsigned long) p.t_configure, (unsigned long) p.t_frame);

  wl_buffer_destroy (buf);
  zwlr_layer_surface_v1_destroy (layer);
  wl_surface_destroy (surface);
  wl_display_disconnect (display);
  return 0;
}
