/* cc-gnoblin-panel.c
 *
 * gnoblin panel for gnome-control-center — the macOS-style "system settings"
 * face of the org.gnoblin.Shell control protocol:
 *
 *   - Feature toggles: ListFeatures() -> a(ssb), one AdwSwitchRow per feature;
 *     flipping a row calls SetFeature(id, bool).
 *   - Screencast grants: ListScreencastGrants() -> as, one AdwActionRow per
 *     granted app with a Revoke button -> RevokeScreencastGrant(id). This is the
 *     equivalent of macOS "Screen Recording" privacy pane.
 *   - "Reload gnoblin": Reload() -> soft in-process shell reload.
 *
 * The rows are built at runtime from the live bus, so the .ui only carries the
 * static scaffold (two AdwPreferencesGroups + a reload button).
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "config.h"

#include <adwaita.h>
#include <glib/gi18n-lib.h>
#include <gio/gio.h>

#include "cc-gnoblin-panel.h"

#define GNOBLIN_BUS_NAME    "org.gnoblin.Shell"
#define GNOBLIN_OBJECT_PATH "/org/gnoblin/Shell"
#define GNOBLIN_IFACE       "org.gnoblin.Shell"

struct _CcGnoblinPanel
{
  CcPanel parent_instance;

  /* Template children */
  AdwPreferencesGroup *features_group;
  AdwPreferencesGroup *screencast_group;
  GtkWidget           *reload_button;
  GtkWidget           *unavailable_banner;

  GDBusProxy   *proxy;        /* org.gnoblin.Shell, or NULL if the shell is down */
  GCancellable *cancellable;

  GList *feature_rows;        /* GtkWidget* rows currently in features_group */
  GList *grant_rows;          /* GtkWidget* rows currently in screencast_group */

  gboolean applying;          /* re-entrancy guard while we push a SetFeature */
};

G_DEFINE_TYPE (CcGnoblinPanel, cc_gnoblin_panel, CC_TYPE_PANEL)

/* forward decls */
static void reload_features (CcGnoblinPanel *self);
static void reload_grants   (CcGnoblinPanel *self);

/* --- small helpers ------------------------------------------------------- */

static void
remove_rows (AdwPreferencesGroup *group,
             GList              **rows)
{
  for (GList *l = *rows; l; l = l->next)
    adw_preferences_group_remove (group, GTK_WIDGET (l->data));
  g_clear_pointer (rows, g_list_free);
}

/* Fire-and-forget a method with no meaningful return; log failures. */
static void
call_done_cb (GObject      *source,
              GAsyncResult *res,
              gpointer      user_data)
{
  g_autoptr(GError) error = NULL;
  g_autoptr(GVariant) ret = NULL;

  ret = g_dbus_proxy_call_finish (G_DBUS_PROXY (source), res, &error);
  if (error && !g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
    g_warning ("gnoblin: D-Bus call failed: %s", error->message);
}

/* --- feature toggles ----------------------------------------------------- */

static void
on_feature_row_activated (AdwSwitchRow *row,
                          GParamSpec   *pspec,
                          gpointer      user_data)
{
  CcGnoblinPanel *self = CC_GNOBLIN_PANEL (user_data);
  const char *id;
  gboolean enabled;

  if (self->applying || self->proxy == NULL)
    return;

  id = g_object_get_data (G_OBJECT (row), "gnoblin-feature-id");
  enabled = adw_switch_row_get_active (row);
  if (id == NULL)
    return;

  g_dbus_proxy_call (self->proxy,
                     "SetFeature",
                     g_variant_new ("(sb)", id, enabled),
                     G_DBUS_CALL_FLAGS_NONE, -1,
                     self->cancellable, call_done_cb, self);
}

static void
list_features_cb (GObject      *source,
                  GAsyncResult *res,
                  gpointer      user_data)
{
  CcGnoblinPanel *self;
  g_autoptr(GError) error = NULL;
  g_autoptr(GVariant) ret = NULL;
  g_autoptr(GVariantIter) iter = NULL;
  const char *id, *summary;
  gboolean enabled;

  ret = g_dbus_proxy_call_finish (G_DBUS_PROXY (source), res, &error);
  if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
    return;

  self = CC_GNOBLIN_PANEL (user_data);

  if (error != NULL)
    {
      g_warning ("gnoblin: ListFeatures failed: %s", error->message);
      return;
    }

  self->applying = TRUE;
  g_variant_get (ret, "(a(ssb))", &iter);
  while (g_variant_iter_loop (iter, "(ssb)", &id, &summary, &enabled))
    {
      GtkWidget *row = adw_switch_row_new ();

      adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row), id);
      adw_action_row_set_subtitle (ADW_ACTION_ROW (row), summary);
      adw_switch_row_set_active (ADW_SWITCH_ROW (row), enabled);
      g_object_set_data_full (G_OBJECT (row), "gnoblin-feature-id",
                              g_strdup (id), g_free);
      g_signal_connect (row, "notify::active",
                        G_CALLBACK (on_feature_row_activated), self);

      adw_preferences_group_add (self->features_group, row);
      self->feature_rows = g_list_prepend (self->feature_rows, row);
    }
  self->applying = FALSE;
}

static void
reload_features (CcGnoblinPanel *self)
{
  remove_rows (self->features_group, &self->feature_rows);
  if (self->proxy == NULL)
    return;

  g_dbus_proxy_call (self->proxy, "ListFeatures", NULL,
                     G_DBUS_CALL_FLAGS_NONE, -1,
                     self->cancellable, list_features_cb, self);
}

/* --- screencast grants --------------------------------------------------- */

static void
on_revoke_clicked (GtkButton *button,
                   gpointer   user_data)
{
  CcGnoblinPanel *self = CC_GNOBLIN_PANEL (user_data);
  const char *id;

  if (self->proxy == NULL)
    return;

  id = g_object_get_data (G_OBJECT (button), "gnoblin-grant-id");
  if (id == NULL)
    return;

  g_dbus_proxy_call (self->proxy,
                     "RevokeScreencastGrant",
                     g_variant_new ("(s)", id),
                     G_DBUS_CALL_FLAGS_NONE, -1,
                     self->cancellable, call_done_cb, self);

  /* Optimistically drop it from the list; a fresh ListScreencastGrants would
   * also work but this keeps the UI snappy. */
  reload_grants (self);
}

static void
list_grants_cb (GObject      *source,
                GAsyncResult *res,
                gpointer      user_data)
{
  CcGnoblinPanel *self;
  g_autoptr(GError) error = NULL;
  g_autoptr(GVariant) ret = NULL;
  g_autoptr(GVariant) array = NULL;
  g_autofree const char **ids = NULL;
  gsize n = 0;

  ret = g_dbus_proxy_call_finish (G_DBUS_PROXY (source), res, &error);
  if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
    return;

  self = CC_GNOBLIN_PANEL (user_data);

  if (error != NULL)
    {
      g_warning ("gnoblin: ListScreencastGrants failed: %s", error->message);
      return;
    }

  /* ids' strings are owned by `array`; keep it alive (g_autoptr) until scope end,
   * which is after the loop below consumes them. */
  array = g_variant_get_child_value (ret, 0);
  ids = g_variant_get_strv (array, &n);

  if (n == 0)
    {
      GtkWidget *row = adw_action_row_new ();
      adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row),
                                     _("No apps have screen-recording access"));
      gtk_widget_set_sensitive (row, FALSE);
      adw_preferences_group_add (self->screencast_group, row);
      self->grant_rows = g_list_prepend (self->grant_rows, row);
      return;
    }

  for (gsize i = 0; i < n; i++)
    {
      GtkWidget *row = adw_action_row_new ();
      GtkWidget *button;

      adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row), ids[i]);

      button = gtk_button_new_with_label (_("Revoke"));
      gtk_widget_set_valign (button, GTK_ALIGN_CENTER);
      gtk_widget_add_css_class (button, "destructive-action");
      g_object_set_data_full (G_OBJECT (button), "gnoblin-grant-id",
                              g_strdup (ids[i]), g_free);
      g_signal_connect (button, "clicked", G_CALLBACK (on_revoke_clicked), self);

      adw_action_row_add_suffix (ADW_ACTION_ROW (row), button);
      adw_preferences_group_add (self->screencast_group, row);
      self->grant_rows = g_list_prepend (self->grant_rows, row);
    }
}

static void
reload_grants (CcGnoblinPanel *self)
{
  remove_rows (self->screencast_group, &self->grant_rows);
  if (self->proxy == NULL)
    return;

  g_dbus_proxy_call (self->proxy, "ListScreencastGrants", NULL,
                     G_DBUS_CALL_FLAGS_NONE, -1,
                     self->cancellable, list_grants_cb, self);
}

/* --- reload button ------------------------------------------------------- */

static void
on_reload_clicked (GtkButton *button,
                   gpointer   user_data)
{
  CcGnoblinPanel *self = CC_GNOBLIN_PANEL (user_data);

  if (self->proxy == NULL)
    return;

  g_dbus_proxy_call (self->proxy, "Reload", NULL,
                     G_DBUS_CALL_FLAGS_NONE, -1,
                     self->cancellable, call_done_cb, self);
}

/* --- proxy lifecycle ----------------------------------------------------- */

static void
proxy_ready_cb (GObject      *source,
                GAsyncResult *res,
                gpointer      user_data)
{
  CcGnoblinPanel *self;
  g_autoptr(GError) error = NULL;
  GDBusProxy *proxy;
  g_autofree char *owner = NULL;

  proxy = g_dbus_proxy_new_for_bus_finish (res, &error);
  if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
    return;

  self = CC_GNOBLIN_PANEL (user_data);

  if (proxy == NULL)
    {
      g_warning ("gnoblin: could not connect to %s: %s",
                 GNOBLIN_BUS_NAME, error ? error->message : "unknown");
      gtk_widget_set_visible (self->unavailable_banner, TRUE);
      return;
    }

  self->proxy = proxy;

  owner = g_dbus_proxy_get_name_owner (proxy);
  if (owner == NULL)
    {
      /* Interface not currently owned — gnoblin shell not running. */
      gtk_widget_set_visible (self->unavailable_banner, TRUE);
      gtk_widget_set_sensitive (self->reload_button, FALSE);
      return;
    }

  gtk_widget_set_visible (self->unavailable_banner, FALSE);
  reload_features (self);
  reload_grants (self);
}

/* --- GObject boilerplate ------------------------------------------------- */

static void
cc_gnoblin_panel_dispose (GObject *object)
{
  CcGnoblinPanel *self = CC_GNOBLIN_PANEL (object);

  if (self->cancellable)
    g_cancellable_cancel (self->cancellable);
  g_clear_object (&self->cancellable);
  g_clear_object (&self->proxy);
  g_clear_pointer (&self->feature_rows, g_list_free);
  g_clear_pointer (&self->grant_rows, g_list_free);

  G_OBJECT_CLASS (cc_gnoblin_panel_parent_class)->dispose (object);
}

static void
cc_gnoblin_panel_class_init (CcGnoblinPanelClass *klass)
{
  GObjectClass *object_class = G_OBJECT_CLASS (klass);
  GtkWidgetClass *widget_class = GTK_WIDGET_CLASS (klass);

  object_class->dispose = cc_gnoblin_panel_dispose;

  gtk_widget_class_set_template_from_resource (widget_class,
    "/org/gnome/control-center/gnoblin/cc-gnoblin-panel.ui");

  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, features_group);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, screencast_group);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, reload_button);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, unavailable_banner);

  gtk_widget_class_bind_template_callback (widget_class, on_reload_clicked);
}

static void
cc_gnoblin_panel_init (CcGnoblinPanel *self)
{
  gtk_widget_init_template (GTK_WIDGET (self));

  self->cancellable = g_cancellable_new ();

  g_dbus_proxy_new_for_bus (G_BUS_TYPE_SESSION,
                            G_DBUS_PROXY_FLAGS_NONE,
                            NULL,
                            GNOBLIN_BUS_NAME,
                            GNOBLIN_OBJECT_PATH,
                            GNOBLIN_IFACE,
                            self->cancellable,
                            proxy_ready_cb,
                            self);
}
