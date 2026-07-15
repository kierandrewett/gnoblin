#define _POSIX_C_SOURCE 200809L

/* Black-box client harness for Gnoblin-owned Wayland protocol boundaries. */
#include "wlr-foreign-toplevel-management-unstable-v1-client-protocol.h"
#include "wlr-layer-shell-unstable-v1-client-protocol.h"
#include "wlr-screencopy-unstable-v1-client-protocol.h"

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <wayland-client.h>

struct protocols
{
  struct zwlr_foreign_toplevel_manager_v1 *foreign_toplevel;
  struct zwlr_layer_shell_v1 *layer_shell;
  struct zwlr_screencopy_manager_v1 *screencopy;
  struct wl_output *output;
  struct wl_shm *shm;
  struct wl_compositor *compositor;
};

static uint32_t
supported_version (uint32_t advertised, uint32_t supported)
{
  return advertised < supported ? advertised : supported;
}
struct foreign_toplevel_state
{
  bool finished;
};

static void
foreign_toplevel (void *data,
                  struct zwlr_foreign_toplevel_manager_v1 *manager,
                  struct zwlr_foreign_toplevel_handle_v1 *toplevel)
{
  (void) data;
  (void) manager;
  (void) toplevel;
}

static void
foreign_finished (void *data,
                  struct zwlr_foreign_toplevel_manager_v1 *manager)
{
  struct foreign_toplevel_state *state = data;

  (void) manager;
  state->finished = true;
}

static const struct zwlr_foreign_toplevel_manager_v1_listener foreign_listener = {
  .toplevel = foreign_toplevel,
  .finished = foreign_finished,
};

struct layer_surface_state
{
  bool configured;
  uint32_t serial;
  uint32_t width;
  uint32_t height;
};

static void
layer_configure (void *data,
                 struct zwlr_layer_surface_v1 *layer_surface,
                 uint32_t serial,
                 uint32_t width,
                 uint32_t height)
{
  struct layer_surface_state *state = data;

  (void) layer_surface;
  state->configured = true;
  state->serial = serial;
  state->width = width;
  state->height = height;
}

static void
layer_closed (void *data,
              struct zwlr_layer_surface_v1 *layer_surface)
{
  (void) data;
  (void) layer_surface;
}

static const struct zwlr_layer_surface_v1_listener layer_listener = {
  .configure = layer_configure,
  .closed = layer_closed,
};


struct screencopy_frame
{
  bool buffer_received;
  bool failed;
  uint32_t format;
  uint32_t width;
  uint32_t height;
  uint32_t stride;
};

static void
screencopy_buffer (void *data,
                   struct zwlr_screencopy_frame_v1 *frame,
                   uint32_t format,
                   uint32_t width,
                   uint32_t height,
                   uint32_t stride)
{
  struct screencopy_frame *state = data;

  (void) frame;
  state->buffer_received = true;
  state->format = format;
  state->width = width;
  state->height = height;
  state->stride = stride;
}

static void
screencopy_flags (void *data,
                  struct zwlr_screencopy_frame_v1 *frame,
                  uint32_t flags)
{
  (void) data;
  (void) frame;
  (void) flags;
}

static void
screencopy_ready (void *data,
                  struct zwlr_screencopy_frame_v1 *frame,
                  uint32_t tv_sec_hi,
                  uint32_t tv_sec_lo,
                  uint32_t tv_nsec)
{
  (void) data;
  (void) frame;
  (void) tv_sec_hi;
  (void) tv_sec_lo;
  (void) tv_nsec;
}

static void
screencopy_failed (void *data,
                   struct zwlr_screencopy_frame_v1 *frame)
{
  struct screencopy_frame *state = data;

  (void) frame;
  state->failed = true;
}

static void
screencopy_damage (void *data,
                   struct zwlr_screencopy_frame_v1 *frame,
                   uint32_t x,
                   uint32_t y,
                   uint32_t width,
                   uint32_t height)
{
  (void) data;
  (void) frame;
  (void) x;
  (void) y;
  (void) width;
  (void) height;
}

static void
screencopy_linux_dmabuf (void *data,
                         struct zwlr_screencopy_frame_v1 *frame,
                         uint32_t format,
                         uint32_t width,
                         uint32_t height)
{
  (void) data;
  (void) frame;
  (void) format;
  (void) width;
  (void) height;
}

static void
screencopy_buffer_done (void *data,
                        struct zwlr_screencopy_frame_v1 *frame)
{
  (void) data;
  (void) frame;
}

static const struct zwlr_screencopy_frame_v1_listener screencopy_listener = {
  .buffer = screencopy_buffer,
  .flags = screencopy_flags,
  .ready = screencopy_ready,
  .failed = screencopy_failed,
  .damage = screencopy_damage,
  .linux_dmabuf = screencopy_linux_dmabuf,
  .buffer_done = screencopy_buffer_done,
};

static void
registry_global (void *data,
                 struct wl_registry *registry,
                 uint32_t name,
                 const char *interface,
                 uint32_t version)
{
  struct protocols *protocols = data;

  if (strcmp (interface, zwlr_foreign_toplevel_manager_v1_interface.name) == 0)
    protocols->foreign_toplevel =
      wl_registry_bind (registry, name,
                        &zwlr_foreign_toplevel_manager_v1_interface,
                        supported_version (version,
                                           zwlr_foreign_toplevel_manager_v1_interface.version));
  else if (strcmp (interface, zwlr_layer_shell_v1_interface.name) == 0)
    protocols->layer_shell =
      wl_registry_bind (registry, name,
                        &zwlr_layer_shell_v1_interface,
                        supported_version (version, zwlr_layer_shell_v1_interface.version));
  else if (strcmp (interface, zwlr_screencopy_manager_v1_interface.name) == 0)
    protocols->screencopy =
      wl_registry_bind (registry, name,
                        &zwlr_screencopy_manager_v1_interface,
                        supported_version (version,
                                           zwlr_screencopy_manager_v1_interface.version));
  else if (strcmp (interface, wl_output_interface.name) == 0 &&
           !protocols->output)
    protocols->output =
      wl_registry_bind (registry, name, &wl_output_interface,
                        supported_version (version, 4));
  else if (strcmp (interface, wl_shm_interface.name) == 0)
    protocols->shm =
      wl_registry_bind (registry, name, &wl_shm_interface, 1);
  else if (strcmp (interface, wl_compositor_interface.name) == 0)
    protocols->compositor =
      wl_registry_bind (registry, name, &wl_compositor_interface,
                        supported_version (version, 6));
}

static void
registry_global_remove (void *data, struct wl_registry *registry, uint32_t name)
{
  (void) data;
  (void) registry;
  (void) name;
}

static const struct wl_registry_listener registry_listener = {
  .global = registry_global,
  .global_remove = registry_global_remove,
};

static bool
test_foreign_toplevel_stop (struct wl_display *display,
                            struct protocols  *protocols)
{
  struct foreign_toplevel_state state = { 0 };

  zwlr_foreign_toplevel_manager_v1_add_listener (protocols->foreign_toplevel,
                                                  &foreign_listener,
                                                  &state);
  zwlr_foreign_toplevel_manager_v1_stop (protocols->foreign_toplevel);
  if (wl_display_roundtrip (display) < 0 || !state.finished)
    {
      fprintf (stderr,
               "FAIL: foreign-toplevel manager did not finish after stop\n");
      return false;
    }

  /* The destructor event releases the client-side id. A fresh sync normally
   * reuses it, which the server can only accept if it destroyed its matching
   * manager resource after sending finished. */
  if (wl_display_roundtrip (display) < 0)
    {
      fprintf (stderr,
               "FAIL: foreign-toplevel manager resource survived finished\n");
      return false;
    }

  protocols->foreign_toplevel = NULL;
  return true;
}

static bool
test_layer_surface_boundaries (void)
{
  struct protocols protocols = { 0 };
  struct layer_surface_state state = { 0 };
  struct wl_display *display = wl_display_connect (NULL);
  struct wl_registry *registry;
  struct wl_surface *surface;
  struct zwlr_layer_surface_v1 *layer_surface;
  struct wl_shm_pool *pool;
  struct wl_buffer *buffer;
  const struct wl_interface *error_interface = NULL;
  uint32_t error_object_id = 0;
  uint32_t layer_object_id;
  uint32_t protocol_error;
  int roundtrip_result;
  char path[] = "/tmp/gnoblin-layer-XXXXXX";
  int fd = -1;

  if (!display)
    {
      fprintf (stderr, "FAIL: layer boundary client could not connect\n");
      return false;
    }

  registry = wl_display_get_registry (display);
  wl_registry_add_listener (registry, &registry_listener, &protocols);
  if (wl_display_roundtrip (display) < 0 ||
      !protocols.compositor ||
      !protocols.layer_shell ||
      !protocols.output ||
      !protocols.shm)
    {
      fprintf (stderr, "FAIL: layer boundary client missed required globals\n");
      wl_display_disconnect (display);
      return false;
    }

  surface = wl_compositor_create_surface (protocols.compositor);
  layer_surface = zwlr_layer_shell_v1_get_layer_surface (
    protocols.layer_shell,
    surface,
    protocols.output,
    ZWLR_LAYER_SHELL_V1_LAYER_TOP,
    "gnoblin-boundary-test");
  zwlr_layer_surface_v1_add_listener (layer_surface, &layer_listener, &state);
  zwlr_layer_surface_v1_set_anchor (
    layer_surface,
    ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP |
    ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT |
    ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
  zwlr_layer_surface_v1_set_size (layer_surface, 0, 1);
  zwlr_layer_surface_v1_set_margin (layer_surface,
                                    INT32_MAX,
                                    INT32_MAX,
                                    INT32_MAX,
                                    INT32_MAX);
  zwlr_layer_surface_v1_set_exclusive_zone (layer_surface, INT32_MAX);
  wl_surface_commit (surface);
  if (wl_display_roundtrip (display) < 0 ||
      !state.configured ||
      state.width != 1 ||
      state.height != 1)
    {
      fprintf (stderr,
               "FAIL: extreme layer margins produced unsafe geometry\n");
      wl_display_disconnect (display);
      return false;
    }

  fd = mkstemp (path);
  if (fd < 0 || unlink (path) != 0 || ftruncate (fd, 4) != 0)
    {
      fprintf (stderr, "FAIL: could not create layer SHM buffer\n");
      if (fd >= 0)
        close (fd);
      wl_display_disconnect (display);
      return false;
    }
  pool = wl_shm_create_pool (protocols.shm, fd, 4);
  buffer = wl_shm_pool_create_buffer (pool,
                                      0,
                                      1,
                                      1,
                                      4,
                                      WL_SHM_FORMAT_ARGB8888);
  wl_shm_pool_destroy (pool);
  zwlr_layer_surface_v1_ack_configure (layer_surface, state.serial);
  wl_surface_attach (surface, buffer, 0, 0);
  wl_surface_damage_buffer (surface, 0, 0, 1, 1);
  wl_surface_commit (surface);
  if (wl_display_roundtrip (display) < 0)
    {
      fprintf (stderr, "FAIL: extreme exclusive zone killed the compositor\n");
      close (fd);
      wl_display_disconnect (display);
      return false;
    }
  wl_buffer_destroy (buffer);
  zwlr_layer_surface_v1_destroy (layer_surface);
  wl_surface_destroy (surface);
  close (fd);
  if (wl_display_roundtrip (display) < 0)
    {
      fprintf (stderr, "FAIL: valid layer surface did not tear down cleanly\n");
      wl_display_disconnect (display);
      return false;
    }

  surface = wl_compositor_create_surface (protocols.compositor);
  layer_surface = zwlr_layer_shell_v1_get_layer_surface (
    protocols.layer_shell,
    surface,
    protocols.output,
    ZWLR_LAYER_SHELL_V1_LAYER_TOP,
    "gnoblin-boundary-test");
  layer_object_id = wl_proxy_get_id ((struct wl_proxy *) layer_surface);
  zwlr_layer_surface_v1_set_anchor (
    layer_surface,
    ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP |
    ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT |
    ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
  zwlr_layer_surface_v1_set_size (layer_surface, UINT32_MAX, 1);
  wl_surface_commit (surface);

  roundtrip_result = wl_display_roundtrip (display);
  protocol_error = wl_display_get_protocol_error (display,
                                                  &error_interface,
                                                  &error_object_id);
  if (roundtrip_result >= 0 ||
      wl_display_get_error (display) != EPROTO ||
      protocol_error != ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_SIZE ||
      error_interface != &zwlr_layer_surface_v1_interface ||
      error_object_id != layer_object_id)
    {
      fprintf (stderr,
               "FAIL: oversized layer surface produced the wrong protocol error\n");
      wl_display_disconnect (display);
      return false;
    }

  wl_display_disconnect (display);
  return true;
}

static bool
test_screencopy_boundaries (struct wl_display *display,
                            struct protocols  *protocols)
{
  struct screencopy_frame extreme_state = { 0 };
  struct screencopy_frame buffer_state = { 0 };
  struct zwlr_screencopy_frame_v1 *extreme_frame;
  struct zwlr_screencopy_frame_v1 *buffer_frame;
  struct wl_shm_pool *pool;
  struct wl_buffer *buffer;
  const struct wl_interface *error_interface = NULL;
  uint64_t pool_size;
  uint32_t wrong_stride;
  uint32_t error_object_id = 0;
  uint32_t frame_object_id;
  uint32_t protocol_error;
  int roundtrip_result;
  char path[] = "/tmp/gnoblin-screencopy-XXXXXX";
  int fd;

  extreme_frame = zwlr_screencopy_manager_v1_capture_output_region (
    protocols->screencopy,
    0,
    protocols->output,
    INT32_MAX,
    INT32_MAX,
    INT32_MAX,
    INT32_MAX);
  zwlr_screencopy_frame_v1_add_listener (extreme_frame,
                                         &screencopy_listener,
                                         &extreme_state);
  if (wl_display_roundtrip (display) < 0 || !extreme_state.failed)
    {
      fprintf (stderr, "FAIL: extreme screencopy region was not rejected\n");
      return false;
    }
  zwlr_screencopy_frame_v1_destroy (extreme_frame);

  buffer_frame = zwlr_screencopy_manager_v1_capture_output (
    protocols->screencopy,
    0,
    protocols->output);
  zwlr_screencopy_frame_v1_add_listener (buffer_frame,
                                         &screencopy_listener,
                                         &buffer_state);
  if (wl_display_roundtrip (display) < 0 ||
      !buffer_state.buffer_received ||
      buffer_state.failed)
    {
      fprintf (stderr, "FAIL: screencopy buffer geometry was not advertised\n");
      return false;
    }

  if (buffer_state.stride > INT32_MAX - 4)
    {
      fprintf (stderr, "FAIL: advertised screencopy stride is too large\n");
      return false;
    }
  wrong_stride = buffer_state.stride + 4;
  pool_size = (uint64_t) wrong_stride * buffer_state.height;
  if (pool_size == 0 || pool_size > INT32_MAX)
    {
      fprintf (stderr, "FAIL: screencopy test buffer is out of range\n");
      return false;
    }

  fd = mkstemp (path);
  if (fd < 0)
    {
      fprintf (stderr, "FAIL: could not create screencopy SHM file: %s\n",
               strerror (errno));
      return false;
    }
  unlink (path);
  if (ftruncate (fd, (off_t) pool_size) != 0)
    {
      fprintf (stderr, "FAIL: could not size screencopy SHM file: %s\n",
               strerror (errno));
      close (fd);
      return false;
    }

  pool = wl_shm_create_pool (protocols->shm, fd, (int32_t) pool_size);
  buffer = wl_shm_pool_create_buffer (pool,
                                      0,
                                      buffer_state.width,
                                      buffer_state.height,
                                      wrong_stride,
                                      buffer_state.format);
  wl_shm_pool_destroy (pool);
  frame_object_id = wl_proxy_get_id ((struct wl_proxy *) buffer_frame);
  zwlr_screencopy_frame_v1_copy (buffer_frame, buffer);

  roundtrip_result = wl_display_roundtrip (display);
  protocol_error = wl_display_get_protocol_error (display,
                                                  &error_interface,
                                                  &error_object_id);
  if (roundtrip_result >= 0 ||
      wl_display_get_error (display) != EPROTO ||
      protocol_error != ZWLR_SCREENCOPY_FRAME_V1_ERROR_INVALID_BUFFER ||
      error_interface != &zwlr_screencopy_frame_v1_interface ||
      error_object_id != frame_object_id)
    {
      fprintf (stderr,
               "FAIL: wrong screencopy stride produced the wrong protocol error\n");
      close (fd);
      return false;
    }

  close (fd);
  return true;
}

int
main (void)
{
  struct protocols protocols = { 0 };
  struct wl_display *display = wl_display_connect (NULL);
  struct wl_registry *registry;

  if (!display)
    {
      fprintf (stderr, "FAIL: could not connect to Wayland display\n");
      return 1;
    }

  registry = wl_display_get_registry (display);
  wl_registry_add_listener (registry, &registry_listener, &protocols);
  if (wl_display_roundtrip (display) < 0)
    {
      fprintf (stderr, "FAIL: registry round-trip failed\n");
      return 1;
    }

  if (!protocols.foreign_toplevel ||
      !protocols.layer_shell ||
      !protocols.screencopy ||
      !protocols.output ||
      !protocols.shm)
    {
      fprintf (stderr,
               "FAIL: missing protocol: foreign=%d layer=%d screencopy=%d output=%d shm=%d\n",
               protocols.foreign_toplevel != NULL,
               protocols.layer_shell != NULL,
               protocols.screencopy != NULL,
               protocols.output != NULL,
               protocols.shm != NULL);
      return 1;
    }

  if (wl_display_roundtrip (display) < 0)
    {
      fprintf (stderr, "FAIL: protocol bind round-trip failed\n");
      return 1;
    }

  if (!test_layer_surface_boundaries ())
    return 1;

  if (!test_foreign_toplevel_stop (display, &protocols))
    return 1;

  if (!test_screencopy_boundaries (display, &protocols))
    return 1;

  wl_display_disconnect (display);
  printf ("PASS: protocol manager lifecycle and geometry boundaries hold\n");
  return 0;
}
