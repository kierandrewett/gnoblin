/* cc-gnoblin-panel.c
 *
 * gnoblin panel for gnome-control-center — the macOS-style "system settings"
 * face of the org.gnoblin.Shell control protocol:
 *
 *   - Feature toggles: ListFeatures() -> a(ssb), one AdwSwitchRow per feature;
 *     flipping a row calls SetFeature(id, bool).
 *   - Portal grants: ListPortalGrants() -> a(sssubb), one AdwActionRow per
 *     persistent Screen Cast or Remote Desktop grant with a Revoke button ->
 *     RevokePortalGrant(portal, id).
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
#include "cc-gnoblin-resources.h"

#define GNOBLIN_BUS_NAME    "org.gnoblin.Shell"
#define GNOBLIN_OBJECT_PATH "/org/gnoblin/Shell"
#define GNOBLIN_IFACE       "org.gnoblin.Shell"

#define GNOBLIN_REMOTE_DEVICE_KEYBOARD    (1u << 0)
#define GNOBLIN_REMOTE_DEVICE_POINTER     (1u << 1)
#define GNOBLIN_REMOTE_DEVICE_TOUCHSCREEN (1u << 2)

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
  GList *grant_rows;          /* GtkWidget* portal rows in screencast_group */

  guint feature_list_serial;
  guint grant_list_serial;
  gboolean disposed;
};

G_DEFINE_TYPE (CcGnoblinPanel, cc_gnoblin_panel, CC_TYPE_PANEL)

typedef struct
{
  CcGnoblinPanel *self;
  GtkWidget *widget;
} PendingCall;

typedef struct
{
  CcGnoblinPanel *self;
  guint serial;
} ListCall;

/* forward decls */
static void reload_features (CcGnoblinPanel *self);
static void reload_grants   (CcGnoblinPanel *self);

static PendingCall *
pending_call_new (CcGnoblinPanel *self,
                  GtkWidget      *widget)
{
  PendingCall *call = g_new0 (PendingCall, 1);

  call->self = g_object_ref (self);
  call->widget = g_object_ref (widget);

  return call;
}

static ListCall *
list_call_new (CcGnoblinPanel *self,
               guint           serial)
{
  ListCall *call = g_new0 (ListCall, 1);

  call->self = g_object_ref (self);
  call->serial = serial;

  return call;
}

/* --- small helpers ------------------------------------------------------- */

static void
remove_rows (AdwPreferencesGroup *group,
             GList              **rows)
{
  for (GList *l = *rows; l; l = l->next)
    adw_preferences_group_remove (group, GTK_WIDGET (l->data));
  g_clear_pointer (rows, g_list_free);
}

/* Reflect whether org.gnoblin.Shell is currently owned: banner + reload button
 * sensitivity + (re)populate or clear the rows. Called on proxy-ready and whenever
 * the name owner changes (shell started/stopped while the panel is open). */
static void
update_availability (CcGnoblinPanel *self)
{
  g_autofree char *owner = self->proxy ? g_dbus_proxy_get_name_owner (self->proxy)
                                       : NULL;
  gboolean up = (owner != NULL);

  adw_banner_set_revealed (ADW_BANNER (self->unavailable_banner), !up);
  gtk_widget_set_sensitive (self->reload_button, up);

  if (up)
    {
      reload_features (self);
      reload_grants (self);
    }
  else
    {
      self->feature_list_serial++;
      self->grant_list_serial++;
      remove_rows (self->features_group, &self->feature_rows);
      remove_rows (self->screencast_group, &self->grant_rows);
    }
}

static void
on_name_owner_changed (GObject    *proxy,
                       GParamSpec *pspec,
                       gpointer    user_data)
{
  CcGnoblinPanel *self = CC_GNOBLIN_PANEL (user_data);

  if (self->disposed)
    return;

  update_availability (self);
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

static AdwSwitchRow *
find_feature_row (CcGnoblinPanel *self,
                  const char     *id)
{
  for (GList *l = self->feature_rows; l; l = l->next)
    {
      const char *row_id;

      row_id = g_object_get_data (G_OBJECT (l->data), "gnoblin-feature-id");
      if (g_strcmp0 (row_id, id) == 0)
        return ADW_SWITCH_ROW (l->data);
    }

  return NULL;
}

static void
set_feature_row_active (AdwSwitchRow *row,
                        gboolean      enabled)
{
  g_object_set_data (G_OBJECT (row), "gnoblin-feature-syncing",
                     GINT_TO_POINTER (TRUE));
  adw_switch_row_set_active (row, enabled);
  g_object_set_data (G_OBJECT (row), "gnoblin-feature-syncing", NULL);
}

static void
on_proxy_signal (GDBusProxy *proxy,
                 const char *sender_name,
                 const char *signal_name,
                 GVariant   *parameters,
                 gpointer    user_data)
{
  CcGnoblinPanel *self = CC_GNOBLIN_PANEL (user_data);
  AdwSwitchRow *row;
  const char *id;
  gboolean enabled;

  if (self->disposed || !g_str_equal (signal_name, "FeatureChanged"))
    return;

  if (!g_variant_is_of_type (parameters, G_VARIANT_TYPE ("(sb)")))
    {
      g_warning ("gnoblin: FeatureChanged returned unexpected type %s",
                 g_variant_get_type_string (parameters));
      return;
    }

  g_variant_get (parameters, "(&sb)", &id, &enabled);
  row = find_feature_row (self, id);
  if (row != NULL)
    set_feature_row_active (row, enabled);
}

static void
set_feature_done_cb (GObject      *source,
                     GAsyncResult *res,
                     gpointer      user_data)
{
  g_autofree PendingCall *call = user_data;
  g_autoptr(CcGnoblinPanel) self = g_steal_pointer (&call->self);
  g_autoptr(GtkWidget) row = g_steal_pointer (&call->widget);
  g_autoptr(GError) error = NULL;
  g_autoptr(GVariant) ret = NULL;

  ret = g_dbus_proxy_call_finish (G_DBUS_PROXY (source), res, &error);
  if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED) ||
      self->disposed)
    return;

  gtk_widget_set_sensitive (row, TRUE);

  if (error != NULL)
    {
      g_warning ("gnoblin: SetFeature failed: %s", error->message);
      reload_features (self);
    }
}

static void
on_feature_row_activated (AdwSwitchRow *row,
                          GParamSpec   *pspec,
                          gpointer      user_data)
{
  CcGnoblinPanel *self = CC_GNOBLIN_PANEL (user_data);
  const char *id;
  gboolean enabled;

  if (self->proxy == NULL ||
      g_object_get_data (G_OBJECT (row), "gnoblin-feature-syncing") != NULL)
    return;

  id = g_object_get_data (G_OBJECT (row), "gnoblin-feature-id");
  enabled = adw_switch_row_get_active (row);
  if (id == NULL)
    return;

  gtk_widget_set_sensitive (GTK_WIDGET (row), FALSE);
  g_dbus_proxy_call (self->proxy,
                     "SetFeature",
                     g_variant_new ("(sb)", id, enabled),
                     G_DBUS_CALL_FLAGS_NONE, -1,
                     self->cancellable, set_feature_done_cb,
                     pending_call_new (self, GTK_WIDGET (row)));
}

static void
list_features_cb (GObject      *source,
                  GAsyncResult *res,
                  gpointer      user_data)
{
  g_autofree ListCall *call = user_data;
  g_autoptr(CcGnoblinPanel) self = g_steal_pointer (&call->self);
  g_autoptr(GError) error = NULL;
  g_autoptr(GVariant) ret = NULL;
  g_autoptr(GVariantIter) iter = NULL;
  const char *id, *summary;
  gboolean enabled;

  ret = g_dbus_proxy_call_finish (G_DBUS_PROXY (source), res, &error);
  if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
    return;
  if (self->disposed || call->serial != self->feature_list_serial)
    return;

  if (error != NULL)
    {
      g_warning ("gnoblin: ListFeatures failed: %s", error->message);
      return;
    }

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
}

static void
reload_features (CcGnoblinPanel *self)
{
  ListCall *call;

  self->feature_list_serial++;
  remove_rows (self->features_group, &self->feature_rows);
  if (self->proxy == NULL || self->disposed)
    return;

  call = list_call_new (self, self->feature_list_serial);
  g_dbus_proxy_call (self->proxy, "ListFeatures", NULL,
                     G_DBUS_CALL_FLAGS_NONE, -1,
                     self->cancellable, list_features_cb, call);
}

/* --- portal grants ------------------------------------------------------- */

static void
revoke_grant_done_cb (GObject      *source,
                      GAsyncResult *res,
                      gpointer      user_data)
{
  g_autofree PendingCall *call = user_data;
  g_autoptr(CcGnoblinPanel) self = g_steal_pointer (&call->self);
  g_autoptr(GtkWidget) button = g_steal_pointer (&call->widget);
  g_autoptr(GError) error = NULL;
  g_autoptr(GVariant) ret = NULL;

  ret = g_dbus_proxy_call_finish (G_DBUS_PROXY (source), res, &error);
  if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED) ||
      self->disposed)
    return;

  if (error != NULL)
    {
      gtk_widget_set_sensitive (button, TRUE);
      g_warning ("gnoblin: RevokePortalGrant failed: %s", error->message);
      return;
    }

  reload_grants (self);
}

static void
on_revoke_clicked (GtkButton *button,
                   gpointer   user_data)
{
  CcGnoblinPanel *self = CC_GNOBLIN_PANEL (user_data);
  GVariant *grant;
  const char *portal;
  const char *id;

  if (self->proxy == NULL)
    return;

  grant = g_object_get_data (G_OBJECT (button), "gnoblin-portal-grant");
  if (grant == NULL)
    return;

  gtk_widget_set_sensitive (GTK_WIDGET (button), FALSE);
  g_variant_get (grant, "(&s&s)", &portal, &id);
  g_dbus_proxy_call (self->proxy,
                     "RevokePortalGrant",
                     g_variant_new ("(ss)", portal, id),
                     G_DBUS_CALL_FLAGS_NONE, -1,
                     self->cancellable, revoke_grant_done_cb,
                     pending_call_new (self, GTK_WIDGET (button)));
}

static void
list_grants_cb (GObject      *source,
                GAsyncResult *res,
                gpointer      user_data)
{
  g_autofree ListCall *call = user_data;
  g_autoptr(CcGnoblinPanel) self = g_steal_pointer (&call->self);
  g_autoptr(GError) error = NULL;
  g_autoptr(GVariant) ret = NULL;
  g_autoptr(GVariantIter) iter = NULL;
  const char *id;
  const char *portal;
  const char *requester;
  guint32 remote_devices;
  gboolean clipboard_enabled;
  gboolean has_screen_streams;

  ret = g_dbus_proxy_call_finish (G_DBUS_PROXY (source), res, &error);
  if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
    return;
  if (self->disposed || call->serial != self->grant_list_serial)
    return;

  if (error != NULL)
    {
      g_warning ("gnoblin: ListPortalGrants failed: %s", error->message);
      return;
    }

  if (!g_variant_is_of_type (ret, G_VARIANT_TYPE ("(a(sssubb))")))
    {
      g_warning ("gnoblin: ListPortalGrants returned unexpected type %s",
                 g_variant_get_type_string (ret));
      return;
    }

  g_variant_get (ret, "(a(sssubb))", &iter);
  if (g_variant_iter_n_children (iter) == 0)
    {
      GtkWidget *row = adw_action_row_new ();
      adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row),
                                     _("No apps have Screen Cast or Remote Desktop access"));
      gtk_widget_set_sensitive (row, FALSE);
      adw_preferences_group_add (self->screencast_group, row);
      self->grant_rows = g_list_prepend (self->grant_rows, row);
      return;
    }

  while (g_variant_iter_loop (iter, "(&s&s&subb)",
                              &id, &portal, &requester,
                              &remote_devices, &clipboard_enabled,
                              &has_screen_streams))
    {
      g_autofree char *subtitle = NULL;
      const char *title = requester;
      GtkWidget *row = adw_action_row_new ();
      GtkWidget *button;

      if (g_str_has_prefix (requester, "app-id:"))
        title += sizeof ("app-id:") - 1;
      else if (g_str_has_prefix (requester, "host-exe:"))
        title += sizeof ("host-exe:") - 1;

      if (g_str_equal (portal, "screen-cast"))
        subtitle = g_strdup (_("Screen Cast"));
      else if (g_str_equal (portal, "remote-desktop"))
        {
          char *capabilities[6] = { NULL, };
          guint n_capabilities = 0;

          if (has_screen_streams)
            capabilities[n_capabilities++] = _("Screen");
          if (remote_devices & GNOBLIN_REMOTE_DEVICE_KEYBOARD)
            capabilities[n_capabilities++] = _("Keyboard");
          if (remote_devices & GNOBLIN_REMOTE_DEVICE_POINTER)
            capabilities[n_capabilities++] = _("Pointer");
          if (remote_devices & GNOBLIN_REMOTE_DEVICE_TOUCHSCREEN)
            capabilities[n_capabilities++] = _("Touchscreen");
          if (clipboard_enabled)
            capabilities[n_capabilities++] = _("Clipboard");

          if (n_capabilities > 0)
            {
              g_autofree char *capability_list = g_strjoinv (", ", capabilities);
              subtitle = g_strdup_printf (_("Remote Desktop: %s"), capability_list);
            }
          else
            subtitle = g_strdup (_("Remote Desktop"));
        }
      else
        subtitle = g_strdup (portal);

      adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row), title);
      adw_action_row_set_subtitle (ADW_ACTION_ROW (row), subtitle);

      button = gtk_button_new_with_label (_("Revoke"));
      gtk_widget_set_valign (button, GTK_ALIGN_CENTER);
      gtk_widget_add_css_class (button, "destructive-action");
      g_object_set_data_full (G_OBJECT (button), "gnoblin-portal-grant",
                              g_variant_ref_sink (g_variant_new ("(ss)", portal, id)),
                              (GDestroyNotify) g_variant_unref);
      g_signal_connect (button, "clicked", G_CALLBACK (on_revoke_clicked), self);

      adw_action_row_add_suffix (ADW_ACTION_ROW (row), button);
      adw_preferences_group_add (self->screencast_group, row);
      self->grant_rows = g_list_prepend (self->grant_rows, row);
    }
}

static void
reload_grants (CcGnoblinPanel *self)
{
  ListCall *call;

  self->grant_list_serial++;
  remove_rows (self->screencast_group, &self->grant_rows);
  if (self->proxy == NULL || self->disposed)
    return;

  call = list_call_new (self, self->grant_list_serial);
  g_dbus_proxy_call (self->proxy, "ListPortalGrants", NULL,
                     G_DBUS_CALL_FLAGS_NONE, -1,
                     self->cancellable, list_grants_cb, call);
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
                     self->cancellable, call_done_cb, NULL);
}

/* --- proxy lifecycle ----------------------------------------------------- */

static void
proxy_ready_cb (GObject      *source,
                GAsyncResult *res,
                gpointer      user_data)
{
  g_autoptr(CcGnoblinPanel) self = user_data;   /* ref taken by the caller */
  g_autoptr(GError) error = NULL;
  GDBusProxy *proxy;

  proxy = g_dbus_proxy_new_for_bus_finish (res, &error);
  if (g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
    return;

  if (self->disposed)
    {
      g_clear_object (&proxy);
      return;
    }

  if (proxy == NULL)
    {
      g_warning ("gnoblin: could not connect to %s: %s",
                 GNOBLIN_BUS_NAME, error ? error->message : "unknown");
      adw_banner_set_revealed (ADW_BANNER (self->unavailable_banner), TRUE);
      return;
    }

  self->proxy = proxy;

  /* Re-evaluate availability whenever the shell starts/stops while we're open. */
  g_signal_connect_object (proxy, "notify::g-name-owner",
                           G_CALLBACK (on_name_owner_changed), self, 0);
  g_signal_connect_object (proxy, "g-signal",
                           G_CALLBACK (on_proxy_signal), self, 0);
  update_availability (self);
}

/* --- GObject boilerplate ------------------------------------------------- */

static void
cc_gnoblin_panel_dispose (GObject *object)
{
  CcGnoblinPanel *self = CC_GNOBLIN_PANEL (object);

  self->disposed = TRUE;
  self->feature_list_serial++;
  self->grant_list_serial++;

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
  /* The compiled .ui lives in this panel's own gresource. It is built into the
   * panel static_library, so nothing in the shell references it and the linker
   * would drop the auto-register constructor; register it explicitly here (as
   * every other panel does, e.g. cc-mouse-panel.c) before init_template looks
   * it up. */
  g_resources_register (cc_gnoblin_get_resource ());

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
                            g_object_ref (self));
}
