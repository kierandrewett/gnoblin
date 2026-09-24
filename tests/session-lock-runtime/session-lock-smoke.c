#define _POSIX_C_SOURCE 200809L

/* Black-box ext-session-lock-v1 smoke client. Never install this as a locker. */
#include "ext-session-lock-v1-client-protocol.h"

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>

enum mode {
  MODE_PROBE,
  MODE_DESTROY_BEFORE_LOCKED,
  MODE_LOCK_UNLOCK,
  MODE_LOCK_HOLD,
  MODE_SECOND,
  MODE_TAKEOVER,
};

struct state {
  struct wl_display *display;
  struct wl_registry *registry;
  struct wl_compositor *compositor;
  struct wl_shm *shm;
  struct wl_output *output;
  struct ext_session_lock_manager_v1 *manager;
  struct ext_session_lock_v1 *lock;
  struct wl_surface *surface;
  struct ext_session_lock_surface_v1 *lock_surface;
  bool configured;
  bool locked;
  bool finished;
  bool committed;
  bool locked_before_client_commit;
  uint32_t serial, width, height;
};

static void
report_display_error (struct wl_display *display, const char *where)
{
  const struct wl_interface *interface = NULL;
  uint32_t id = 0;
  uint32_t code = wl_display_get_protocol_error (display, &interface, &id);
  int error = wl_display_get_error (display);

  fprintf (stderr, "%s: display error=%d protocol=%s#%u code=%u\n", where,
           error, interface ? interface->name : "none", id, code);
}

static int
wait_until (struct state *state, bool *condition, int seconds)
{
  struct timespec start, now;
  int64_t deadline_ms;
  clock_gettime (CLOCK_MONOTONIC, &start);
  deadline_ms = (int64_t) start.tv_sec * 1000 + start.tv_nsec / 1000000 + seconds * 1000;
  while (!*condition)
    {
      struct pollfd poll_fd = { .fd = wl_display_get_fd (state->display), .events = POLLIN };
      int64_t now_ms;
      int timeout_ms;

      if (wl_display_dispatch_pending (state->display) < 0)
        { report_display_error (state->display, "pending event dispatch"); return -1; }
      if (*condition)
        break;
      clock_gettime (CLOCK_MONOTONIC, &now);
      now_ms = (int64_t) now.tv_sec * 1000 + now.tv_nsec / 1000000;
      if (now_ms >= deadline_ms)
        return 0;
      timeout_ms = (int) (deadline_ms - now_ms);
      if (wl_display_flush (state->display) < 0 && errno != EAGAIN)
        { report_display_error (state->display, "display flush"); return -1; }
      if (poll (&poll_fd, 1, timeout_ms) < 0)
        { perror ("poll"); return -1; }
      if (poll_fd.revents == 0)
        return 0;
      if (wl_display_dispatch (state->display) < 0)
        { report_display_error (state->display, "event dispatch"); return -1; }
    }
  return 1;
}

static void
lock_locked (void *data, struct ext_session_lock_v1 *lock)
{
  struct state *state = data;
  (void) lock;
  if (!state->committed)
    state->locked_before_client_commit = true;
  state->locked = true;
}

static void
lock_finished (void *data, struct ext_session_lock_v1 *lock)
{
  struct state *state = data;
  (void) lock;
  state->finished = true;
}

static const struct ext_session_lock_v1_listener lock_listener = {
  .locked = lock_locked,
  .finished = lock_finished,
};

static void
surface_configure (void *data, struct ext_session_lock_surface_v1 *surface,
                   uint32_t serial, uint32_t width, uint32_t height)
{
  struct state *state = data;
  (void) surface;
  state->configured = true;
  state->serial = serial;
  state->width = width;
  state->height = height;
}

static const struct ext_session_lock_surface_v1_listener surface_listener = {
  .configure = surface_configure,
};

static void
registry_global (void *data, struct wl_registry *registry, uint32_t name,
                 const char *interface, uint32_t version)
{
  struct state *state = data;
  if (strcmp (interface, wl_compositor_interface.name) == 0)
    state->compositor = wl_registry_bind (registry, name, &wl_compositor_interface,
                                           version < 4 ? version : 4);
  else if (strcmp (interface, wl_shm_interface.name) == 0)
    state->shm = wl_registry_bind (registry, name, &wl_shm_interface, 1);
  else if (!state->output && strcmp (interface, wl_output_interface.name) == 0)
    state->output = wl_registry_bind (registry, name, &wl_output_interface,
                                       version < 2 ? version : 2);
  else if (strcmp (interface, ext_session_lock_manager_v1_interface.name) == 0)
    state->manager = wl_registry_bind (registry, name,
                                       &ext_session_lock_manager_v1_interface, 1);
}

static void registry_remove (void *data, struct wl_registry *registry, uint32_t name)
{ (void) data; (void) registry; (void) name; }

static const struct wl_registry_listener registry_listener = {
  .global = registry_global, .global_remove = registry_remove,
};

static int
create_buffer (struct state *state)
{
  char filename[] = "/tmp/gnoblin-session-lock.XXXXXX";
  int fd = mkstemp (filename);
  void *pixels;
  size_t size;
  struct wl_shm_pool *pool;
  struct wl_buffer *buffer;
  uint32_t width = state->width ? state->width : 1;
  uint32_t height = state->height ? state->height : 1;

  if (fd < 0)
    return -1;
  unlink (filename);
  size = (size_t) width * height * 4;
  if (ftruncate (fd, (off_t) size) != 0)
    { close (fd); return -1; }
  pixels = mmap (NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  if (pixels == MAP_FAILED)
    { close (fd); return -1; }
  memset (pixels, 0, size); /* opaque black in XRGB8888 */
  pool = wl_shm_create_pool (state->shm, fd, (int) size);
  buffer = wl_shm_pool_create_buffer (pool, 0, (int) width, (int) height,
                                      (int) width * 4, WL_SHM_FORMAT_XRGB8888);
  wl_shm_pool_destroy (pool);
  close (fd);
  ext_session_lock_surface_v1_ack_configure (state->lock_surface, state->serial);
  wl_surface_attach (state->surface, buffer, 0, 0);
  wl_surface_damage_buffer (state->surface, 0, 0, INT32_MAX, INT32_MAX);
  wl_surface_commit (state->surface);
  wl_buffer_destroy (buffer);
  munmap (pixels, size);
  state->committed = true;
  return wl_display_flush (state->display) < 0 && errno != EAGAIN ? -1 : 0;
}

static int
start_lock (struct state *state)
{
  state->lock = ext_session_lock_manager_v1_lock (state->manager);
  ext_session_lock_v1_add_listener (state->lock, &lock_listener, state);
  state->surface = wl_compositor_create_surface (state->compositor);
  state->lock_surface = ext_session_lock_v1_get_lock_surface (state->lock, state->surface,
                                                               state->output);
  ext_session_lock_surface_v1_add_listener (state->lock_surface, &surface_listener, state);
  if (wl_display_roundtrip (state->display) < 0)
    { report_display_error (state->display, "lock-surface roundtrip"); return -1; }
  if (!state->configured)
    { fputs ("lock surface received no configure\n", stderr); return -1; }
  return create_buffer (state);
}

static enum mode
parse_mode (const char *value)
{
  if (strcmp (value, "probe") == 0) return MODE_PROBE;
  if (strcmp (value, "destroy-before-locked") == 0) return MODE_DESTROY_BEFORE_LOCKED;
  if (strcmp (value, "lock-unlock") == 0) return MODE_LOCK_UNLOCK;
  if (strcmp (value, "lock-hold") == 0) return MODE_LOCK_HOLD;
  if (strcmp (value, "second") == 0) return MODE_SECOND;
  if (strcmp (value, "takeover") == 0) return MODE_TAKEOVER;
  fprintf (stderr, "unknown mode: %s\n", value);
  exit (2);
}

int
main (int argc, char **argv)
{
  struct state state = {0};
  enum mode mode = parse_mode (argc == 2 ? argv[1] : "probe");
  int result;

  state.display = wl_display_connect (NULL);
  if (!state.display) { perror ("wl_display_connect"); return 1; }
  state.registry = wl_display_get_registry (state.display);
  wl_registry_add_listener (state.registry, &registry_listener, &state);
  if (wl_display_roundtrip (state.display) < 0) return 1;
  if (!state.manager) { puts ("SKIP: ext_session_lock_manager_v1 is not advertised"); return 77; }
  if (!state.compositor || !state.shm || !state.output) {
    fputs ("missing wl_compositor, wl_shm, or wl_output\n", stderr); return 1;
  }
  if (mode == MODE_PROBE) { puts ("PROBE: ext_session_lock_manager_v1 available"); return 0; }

  if (mode == MODE_DESTROY_BEFORE_LOCKED)
    {
      /* The standard protocol permits destroy until locked is sent. A fresh
       * request must still be able to acquire the lock afterwards; otherwise
       * a cancelled locker would have left the session spuriously locked. */
      state.lock = ext_session_lock_manager_v1_lock (state.manager);
      ext_session_lock_v1_add_listener (state.lock, &lock_listener, &state);
      ext_session_lock_v1_destroy (state.lock);
      state.lock = NULL;
      if (wl_display_roundtrip (state.display) < 0)
        { fputs ("destroy before locked was rejected\n", stderr); return 1; }
      puts ("CANCELLED: pre-locked lock object destroyed");
      state.locked = false;
      state.finished = false;
      state.locked_before_client_commit = false;
    }

  if (mode == MODE_SECOND) {
    state.lock = ext_session_lock_manager_v1_lock (state.manager);
    ext_session_lock_v1_add_listener (state.lock, &lock_listener, &state);
    result = wait_until (&state, &state.finished, 5);
    if (result != 1 || state.locked) { fputs ("second lock was not finished\n", stderr); return 1; }
    puts ("SECOND: finished"); return 0;
  }

  if (start_lock (&state) != 0) { fputs ("lock surface setup failed\n", stderr); return 1; }
  result = wait_until (&state, &state.locked, 10);
  if (result != 1 || state.finished) { fputs ("lock did not reach presented locked state\n", stderr); return 1; }
  if (state.locked_before_client_commit)
    puts ("LOCKED: compositor presentation confirmed before client buffer (opaque fallback)");
  else
    puts ("LOCKED: compositor presentation confirmed after opaque client buffer commit");
  fflush (stdout);
  if (mode == MODE_LOCK_HOLD)
    {
      /* The runner kills this client after proving a concurrent request gets
       * finished. Do not unlock on ordinary process termination. */
      for (;;) pause ();
    }
  ext_session_lock_v1_unlock_and_destroy (state.lock);
  if (wl_display_roundtrip (state.display) < 0) { fputs ("unlock was rejected\n", stderr); return 1; }
  puts (mode == MODE_TAKEOVER ? "TAKEOVER: unlocked" : "UNLOCKED: server processed request");
  return 0;
}
