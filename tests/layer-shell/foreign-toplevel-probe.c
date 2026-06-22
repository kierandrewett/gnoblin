// Probe client for ext-foreign-toplevel-list-v1: binds the toplevel list and
// prints the app-id of every toplevel the compositor reports, one per line
// (`TOPLEVEL app_id=<id>`). A taskbar / wlrctl / pager relies on this; this
// proves the compositor actually streams the window list, not just advertises
// the global. Used by foreign-toplevel-test.py.
#include <stdio.h>
#include <string.h>
#include <wayland-client.h>
#include "ext-foreign-toplevel-list.h"

static struct ext_foreign_toplevel_list_v1 *list_mgr;
#define MAX 64
static char app_ids[MAX][256];
static int n_ids;

static void h_closed(void *d, struct ext_foreign_toplevel_handle_v1 *h) {}
static void h_done(void *d, struct ext_foreign_toplevel_handle_v1 *h) {}
static void h_title(void *d, struct ext_foreign_toplevel_handle_v1 *h, const char *t) {}
static void h_app_id(void *d, struct ext_foreign_toplevel_handle_v1 *h, const char *app_id) {
  if (n_ids < MAX) { snprintf(app_ids[n_ids], 256, "%s", app_id); n_ids++; }
}
static void h_identifier(void *d, struct ext_foreign_toplevel_handle_v1 *h, const char *id) {}
static const struct ext_foreign_toplevel_handle_v1_listener h_listener = {
  h_closed, h_done, h_title, h_app_id, h_identifier };

static void l_toplevel(void *d, struct ext_foreign_toplevel_list_v1 *l,
                       struct ext_foreign_toplevel_handle_v1 *handle) {
  ext_foreign_toplevel_handle_v1_add_listener(handle, &h_listener, NULL);
}
static void l_finished(void *d, struct ext_foreign_toplevel_list_v1 *l) {}
static const struct ext_foreign_toplevel_list_v1_listener l_listener = {
  l_toplevel, l_finished };

static void reg_global(void *d, struct wl_registry *r, uint32_t name,
                       const char *iface, uint32_t ver) {
  if (!strcmp(iface, ext_foreign_toplevel_list_v1_interface.name))
    list_mgr = wl_registry_bind(r, name, &ext_foreign_toplevel_list_v1_interface, 1);
}
static void reg_remove(void *d, struct wl_registry *r, uint32_t name) {}
static const struct wl_registry_listener reg_listener = { reg_global, reg_remove };

int main(void) {
  struct wl_display *dpy = wl_display_connect(NULL);
  if (!dpy) { fprintf(stderr, "no display\n"); return 2; }
  struct wl_registry *reg = wl_display_get_registry(dpy);
  wl_registry_add_listener(reg, &reg_listener, NULL);
  wl_display_roundtrip(dpy);
  if (!list_mgr) { fprintf(stderr, "no ext_foreign_toplevel_list_v1\n"); return 2; }
  ext_foreign_toplevel_list_v1_add_listener(list_mgr, &l_listener, NULL);
  wl_display_roundtrip(dpy);   // toplevel events -> handles
  wl_display_roundtrip(dpy);   // title/app_id/done events on the handles
  for (int i = 0; i < n_ids; i++)
    printf("TOPLEVEL app_id=%s\n", app_ids[i]);
  wl_display_disconnect(dpy);
  return 0;
}
