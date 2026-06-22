/* cc-gnoblin-panel.c
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "cc-gnoblin-panel.h"

#include "cc-gnoblin-resources.h"

#include <glib/gi18n.h>

#define GNOBLIN_SCHEMA_ID "org.gnoblin.shell"

typedef enum
{
  FEATURE_TOPBAR,
  FEATURE_DOCK,
} GnoblinFeature;

struct _CcGnoblinPanel
{
  CcPanel parent_instance;

  GSettings *settings;
  GListModel *monitors;
  gulong monitors_changed_id;

  AdwPreferencesPage *preferences_page;
  AdwPreferencesGroup *schema_missing_group;

  AdwPreferencesGroup *topbar_group;
  AdwSwitchRow *topbar_row;
  AdwComboRow *topbar_monitor_row;
  AdwPreferencesGroup *topbar_displays_group;

  AdwPreferencesGroup *dock_group;
  AdwSwitchRow *dock_row;
  AdwComboRow *dock_monitor_row;
  AdwPreferencesGroup *dock_displays_group;

  AdwPreferencesGroup *protocols_group;
  AdwSwitchRow *layer_shell_row;
  AdwSwitchRow *screencopy_row;

  gboolean updating;
};

CC_PANEL_REGISTER (CcGnoblinPanel, cc_gnoblin_panel)

static const char *monitor_modes[] =
{
  "primary",
  "all",
  "manual",
};

static gboolean
nonempty (const char *value)
{
  return value != NULL && value[0] != '\0';
}

static GSettings *
gnoblin_settings_new (void)
{
  g_autoptr(GSettingsSchema) schema = NULL;
  GSettingsSchemaSource *source;

  source = g_settings_schema_source_get_default ();
  if (source == NULL)
    return NULL;

  schema = g_settings_schema_source_lookup (source, GNOBLIN_SCHEMA_ID, TRUE);
  if (schema == NULL)
    return NULL;

  return g_settings_new_full (schema, NULL, NULL);
}

static char *
monitor_id (GdkMonitor *monitor,
            guint       index)
{
  const char *connector;
  const char *manufacturer;
  const char *model;
  g_autoptr(GPtrArray) parts = NULL;
  GdkRectangle geometry;

  if (monitor == NULL)
    return g_strdup ("default");

  connector = gdk_monitor_get_connector (monitor);
  if (nonempty (connector))
    return g_strdup (connector);

  parts = g_ptr_array_new ();

  manufacturer = gdk_monitor_get_manufacturer (monitor);
  if (nonempty (manufacturer))
    g_ptr_array_add (parts, (gpointer) manufacturer);

  model = gdk_monitor_get_model (monitor);
  if (nonempty (model))
    g_ptr_array_add (parts, (gpointer) model);

  if (parts->len > 0)
    {
      GString *id = g_string_new (g_ptr_array_index (parts, 0));

      for (guint i = 1; i < parts->len; i++)
        g_string_append_printf (id, ":%s", (const char *) g_ptr_array_index (parts, i));

      return g_string_free (id, FALSE);
    }

  gdk_monitor_get_geometry (monitor, &geometry);
  return g_strdup_printf ("monitor-%u-%d,%d-%dx%d",
                          index,
                          geometry.x,
                          geometry.y,
                          geometry.width,
                          geometry.height);
}

static char *
monitor_title (GdkMonitor *monitor,
               guint       index)
{
  const char *description;
  const char *connector;
  const char *model;

  if (monitor == NULL)
    return g_strdup (_("Default Display"));

  description = gdk_monitor_get_description (monitor);
  if (nonempty (description))
    return g_strdup (description);

  connector = gdk_monitor_get_connector (monitor);
  model = gdk_monitor_get_model (monitor);

  if (nonempty (connector) && nonempty (model))
    return g_strdup_printf ("%s (%s)", model, connector);

  if (nonempty (connector))
    return g_strdup (connector);

  if (nonempty (model))
    return g_strdup (model);

  return g_strdup_printf (_("Display %u"), index + 1);
}

static guint
mode_to_index (const char *mode)
{
  for (guint i = 0; i < G_N_ELEMENTS (monitor_modes); i++)
    {
      if (g_strcmp0 (mode, monitor_modes[i]) == 0)
        return i;
    }

  return 0;
}

static void
clear_group_rows (AdwPreferencesGroup *group)
{
  GtkWidget *row;

  while ((row = adw_preferences_group_get_row (group, 0)) != NULL)
    adw_preferences_group_remove (group, row);
}

static const char *
monitors_key_for_feature (GnoblinFeature feature)
{
  return feature == FEATURE_TOPBAR ? "topbar-monitors" : "dock-monitors";
}

static gboolean
row_is_enabled (CcGnoblinPanel *self,
                GnoblinFeature  feature,
                const char     *id)
{
  g_auto(GStrv) ids = NULL;

  ids = g_settings_get_strv (self->settings, monitors_key_for_feature (feature));
  return g_strv_contains ((const char * const *) ids, id);
}

static void
set_monitor_enabled (CcGnoblinPanel *self,
                     GnoblinFeature  feature,
                     const char     *id,
                     gboolean        enabled)
{
  g_auto(GStrv) ids = NULL;
  g_autoptr(GPtrArray) values = NULL;
  gboolean found = FALSE;

  ids = g_settings_get_strv (self->settings, monitors_key_for_feature (feature));
  values = g_ptr_array_new_with_free_func (g_free);

  for (guint i = 0; ids[i] != NULL; i++)
    {
      if (g_strcmp0 (ids[i], id) == 0)
        {
          found = TRUE;
          if (!enabled)
            continue;
        }

      g_ptr_array_add (values, g_strdup (ids[i]));
    }

  if (enabled && !found)
    g_ptr_array_add (values, g_strdup (id));

  g_ptr_array_add (values, NULL);
  g_settings_set_strv (self->settings,
                       monitors_key_for_feature (feature),
                       (const char * const *) values->pdata);
}

static void
monitor_check_toggled_cb (GtkCheckButton *check,
                          CcGnoblinPanel *self)
{
  GnoblinFeature feature;
  const char *id;

  if (self->updating || self->settings == NULL)
    return;

  id = g_object_get_data (G_OBJECT (check), "gnoblin-monitor-id");
  feature = GPOINTER_TO_INT (g_object_get_data (G_OBJECT (check), "gnoblin-feature"));

  if (id == NULL)
    return;

  set_monitor_enabled (self, feature, id, gtk_check_button_get_active (check));
}

static void
add_monitor_row (CcGnoblinPanel     *self,
                 AdwPreferencesGroup *group,
                 GnoblinFeature       feature,
                 GdkMonitor          *monitor,
                 guint                index)
{
  g_autofree char *id = NULL;
  g_autofree char *title = NULL;
  GtkWidget *row;
  GtkWidget *check;

  id = monitor_id (monitor, index);
  title = monitor_title (monitor, index);

  row = adw_action_row_new ();
  adw_preferences_row_set_title (ADW_PREFERENCES_ROW (row), title);
  adw_action_row_set_subtitle (ADW_ACTION_ROW (row), id);
  adw_action_row_set_subtitle_selectable (ADW_ACTION_ROW (row), TRUE);

  check = gtk_check_button_new ();
  gtk_widget_set_valign (check, GTK_ALIGN_CENTER);
  gtk_check_button_set_active (GTK_CHECK_BUTTON (check), row_is_enabled (self, feature, id));
  g_object_set_data_full (G_OBJECT (check), "gnoblin-monitor-id", g_strdup (id), g_free);
  g_object_set_data (G_OBJECT (check), "gnoblin-feature", GINT_TO_POINTER (feature));
  g_signal_connect (check, "toggled", G_CALLBACK (monitor_check_toggled_cb), self);

  adw_action_row_add_suffix (ADW_ACTION_ROW (row), check);
  adw_action_row_set_activatable_widget (ADW_ACTION_ROW (row), check);
  adw_preferences_group_add (group, row);
}

static void
refresh_monitor_rows (CcGnoblinPanel *self)
{
  guint n_monitors = 0;

  clear_group_rows (self->topbar_displays_group);
  clear_group_rows (self->dock_displays_group);

  if (self->monitors != NULL)
    n_monitors = g_list_model_get_n_items (self->monitors);

  if (n_monitors == 0)
    {
      add_monitor_row (self, self->topbar_displays_group, FEATURE_TOPBAR, NULL, 0);
      add_monitor_row (self, self->dock_displays_group, FEATURE_DOCK, NULL, 0);
      return;
    }

  for (guint i = 0; i < n_monitors; i++)
    {
      g_autoptr(GdkMonitor) monitor = NULL;

      monitor = g_list_model_get_item (self->monitors, i);
      add_monitor_row (self, self->topbar_displays_group, FEATURE_TOPBAR, monitor, i);
      add_monitor_row (self, self->dock_displays_group, FEATURE_DOCK, monitor, i);
    }
}

static void
sync_mode_row (CcGnoblinPanel *self,
               AdwComboRow    *row,
               const char     *settings_key)
{
  g_autofree char *mode = NULL;

  mode = g_settings_get_string (self->settings, settings_key);
  adw_combo_row_set_selected (row, mode_to_index (mode));
}

static void
sync_from_settings (CcGnoblinPanel *self)
{
  g_autofree char *topbar_mode = NULL;
  g_autofree char *dock_mode = NULL;

  if (self->settings == NULL)
    return;

  self->updating = TRUE;

  sync_mode_row (self, self->topbar_monitor_row, "topbar-monitor-mode");
  sync_mode_row (self, self->dock_monitor_row, "dock-monitor-mode");

  topbar_mode = g_settings_get_string (self->settings, "topbar-monitor-mode");
  dock_mode = g_settings_get_string (self->settings, "dock-monitor-mode");

  gtk_widget_set_visible (GTK_WIDGET (self->topbar_displays_group),
                          g_strcmp0 (topbar_mode, "manual") == 0);
  gtk_widget_set_visible (GTK_WIDGET (self->dock_displays_group),
                          g_strcmp0 (dock_mode, "manual") == 0);

  refresh_monitor_rows (self);

  self->updating = FALSE;
}

static void
settings_changed_cb (GSettings      *settings,
                     const char     *key,
                     CcGnoblinPanel *self)
{
  sync_from_settings (self);
}

static void
set_monitor_mode_from_row (CcGnoblinPanel *self,
                           AdwComboRow    *row,
                           const char     *settings_key)
{
  guint selected;

  if (self->updating || self->settings == NULL)
    return;

  selected = adw_combo_row_get_selected (row);
  if (selected >= G_N_ELEMENTS (monitor_modes))
    selected = 0;

  g_settings_set_string (self->settings, settings_key, monitor_modes[selected]);
}

static void
topbar_monitor_mode_row_changed_cb (CcGnoblinPanel *self)
{
  set_monitor_mode_from_row (self, self->topbar_monitor_row, "topbar-monitor-mode");
}

static void
dock_monitor_mode_row_changed_cb (CcGnoblinPanel *self)
{
  set_monitor_mode_from_row (self, self->dock_monitor_row, "dock-monitor-mode");
}

static void
monitors_items_changed_cb (GListModel     *monitors,
                           guint           position,
                           guint           removed,
                           guint           added,
                           CcGnoblinPanel *self)
{
  sync_from_settings (self);
}

static void
setup_monitor_model (CcGnoblinPanel *self)
{
  GdkDisplay *display;
  GListModel *monitors;

  display = gdk_display_get_default ();
  if (display == NULL)
    return;

  monitors = gdk_display_get_monitors (display);
  if (monitors == NULL)
    return;

  self->monitors = g_object_ref (monitors);
  self->monitors_changed_id =
      g_signal_connect (self->monitors,
                        "items-changed",
                        G_CALLBACK (monitors_items_changed_cb),
                        self);
}

static void
cc_gnoblin_panel_finalize (GObject *object)
{
  CcGnoblinPanel *self = CC_GNOBLIN_PANEL (object);

  g_clear_signal_handler (&self->monitors_changed_id, self->monitors);
  g_clear_object (&self->monitors);
  g_clear_object (&self->settings);

  G_OBJECT_CLASS (cc_gnoblin_panel_parent_class)->finalize (object);
}

static void
cc_gnoblin_panel_class_init (CcGnoblinPanelClass *klass)
{
  GObjectClass *object_class = G_OBJECT_CLASS (klass);
  GtkWidgetClass *widget_class = GTK_WIDGET_CLASS (klass);

  object_class->finalize = cc_gnoblin_panel_finalize;

  gtk_widget_class_set_template_from_resource (widget_class, "/org/gnome/control-center/gnoblin/cc-gnoblin-panel.ui");

  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, preferences_page);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, schema_missing_group);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, topbar_group);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, topbar_row);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, topbar_monitor_row);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, topbar_displays_group);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, dock_group);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, dock_row);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, dock_monitor_row);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, dock_displays_group);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, protocols_group);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, layer_shell_row);
  gtk_widget_class_bind_template_child (widget_class, CcGnoblinPanel, screencopy_row);

  gtk_widget_class_bind_template_callback (widget_class, topbar_monitor_mode_row_changed_cb);
  gtk_widget_class_bind_template_callback (widget_class, dock_monitor_mode_row_changed_cb);
}

static void
cc_gnoblin_panel_init (CcGnoblinPanel *self)
{
  g_resources_register (cc_gnoblin_get_resource ());

  gtk_widget_init_template (GTK_WIDGET (self));

  self->settings = gnoblin_settings_new ();
  if (self->settings == NULL)
    {
      gtk_widget_set_visible (GTK_WIDGET (self->schema_missing_group), TRUE);
      gtk_widget_set_sensitive (GTK_WIDGET (self->topbar_group), FALSE);
      gtk_widget_set_sensitive (GTK_WIDGET (self->dock_group), FALSE);
      gtk_widget_set_sensitive (GTK_WIDGET (self->protocols_group), FALSE);
      return;
    }

  g_settings_bind (self->settings,
                   "topbar-enabled",
                   self->topbar_row,
                   "active",
                   G_SETTINGS_BIND_DEFAULT);
  g_settings_bind (self->settings,
                   "dock-enabled",
                   self->dock_row,
                   "active",
                   G_SETTINGS_BIND_DEFAULT);
  g_settings_bind (self->settings,
                   "wlr-layer-shell-enabled",
                   self->layer_shell_row,
                   "active",
                   G_SETTINGS_BIND_DEFAULT);
  g_settings_bind (self->settings,
                   "wlr-screencopy-enabled",
                   self->screencopy_row,
                   "active",
                   G_SETTINGS_BIND_DEFAULT);

  g_signal_connect (self->settings, "changed", G_CALLBACK (settings_changed_cb), self);

  setup_monitor_model (self);
  sync_from_settings (self);
}
