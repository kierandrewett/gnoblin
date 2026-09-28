/*
 * gnoblin: shared configuration loading for Mutter and Shell.
 *
 * Read from $GNOBLIN_CONFIG, else the first existing init.lua, gnoblin.toml,
 * or gnoblin.conf under $XDG_CONFIG_HOME/gnoblin. A fresh installation uses
 * init.lua. `GNOBLIN_CONFIG` can select any supported configuration filename.
 *
 * Missing files and keys use the caller's default. Invalid reloads retain
 * the last valid configuration. Mutter uses these accessors to gate Wayland
 * protocols; the shell receives the same evaluated document for live settings.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

/* Current root filename. Free the result with g_free(). */
char* gnoblin_config_path(void);
GVariant* gnoblin_config_parse_toml(const char* contents, GError** error);
/* Evaluate one config with a fresh Lua state and record every dependency. */
GVariant* gnoblin_config_evaluate_file(const char* path, GPtrArray* paths, GPtrArray* directories,
                                       GError** error);
/* Persistent Lua runtime used by Mutter for configuration event callbacks. */
GVariant* gnoblin_config_load_runtime(const char* path, GPtrArray** paths, GPtrArray** directories,
                                      GError** error);
void gnoblin_config_finish_load(gboolean commit);
/* The committed runtime document; returns a new reference or NULL. */
GVariant* gnoblin_config_current_document(void);
guint64 gnoblin_config_settings_revision(void);
/* Cache the latest native window snapshot for immediate Lua reads. Passing
 * NULL marks the snapshot unavailable until native control refreshes it. */
void gnoblin_config_update_window_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the latest native workspace snapshot for immediate Lua reads. */
void gnoblin_config_update_workspace_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the latest native monitor snapshot for immediate Lua reads. */
void gnoblin_config_update_monitor_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the latest native layer-surface snapshot for immediate Lua reads. */
void gnoblin_config_update_layer_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the latest native capability snapshot for immediate Lua reads. */
void gnoblin_config_update_capability_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the latest native input-device snapshot for immediate Lua reads. */
void gnoblin_config_update_input_device_snapshot(GVariant* snapshot, guint64 revision);
/* Cache configured XKB input sources and the confirmed current source. */
void gnoblin_config_update_input_source_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the configured native shortcut snapshot. */
void gnoblin_config_update_shortcut_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the latest native launch-feedback snapshot for immediate Lua reads. */
void gnoblin_config_update_launch_snapshot(GVariant* snapshot, guint64 revision);
/* Cache validated portal grants for immediate Lua collection reads. */
void gnoblin_config_update_portal_grant_snapshot(GVariant* snapshot, guint64 revision);
/* Notify the native host after a committed change to the effective focus policy. */
typedef void (*GnoblinConfigFocusPolicyChangedFunc)(GVariant* policy, guint64 revision,
                                                    gpointer user_data);
void gnoblin_config_set_focus_policy_changed_callback(GnoblinConfigFocusPolicyChangedFunc callback,
                                                      gpointer user_data);
/* Notify the native host after a committed permission policy change. */
typedef void (*GnoblinConfigPermissionPolicyChangedFunc)(GVariant* policy, guint64 revision,
                                                         gpointer user_data);
void gnoblin_config_set_permission_policy_changed_callback(
    GnoblinConfigPermissionPolicyChangedFunc callback, gpointer user_data);
GVariant* gnoblin_config_dispatch_event(const char* event, GVariant* payload, GError** error);
/* Dispatch a trusted native shortcut event with a private, userdata-only
 * focus context. The context is never added to the public event payload. */
GVariant* gnoblin_config_dispatch_shortcut_event(const char* event, GVariant* payload,
                                                 guint64 context_handle, gint64 expires_at_us,
                                                 GError** error);
guint64 gnoblin_config_runtime_generation(void);
gboolean gnoblin_config_take_focus_context(gint64 request_id, guint64* context_handle,
                                           guint64* generation);
/* The compositor host schedules deferred Lua callbacks on its main loop and
 * dispatches them through the normal config validation/operation path. */
typedef void (*GnoblinConfigRuntimeWakeupFunc)(gpointer user_data);
void gnoblin_config_set_runtime_wakeup_callback(GnoblinConfigRuntimeWakeupFunc callback,
                                                gpointer user_data);
GVariant* gnoblin_config_dispatch_deferred_callbacks(GError** error);
/* Deferred runtime operations use a shared method registry. Event callbacks
 * enqueue operations; the host drains them only after dispatch returns. */
GVariant* gnoblin_config_drain_runtime_operations(void); /* aa{sv} */
GVariant* gnoblin_config_call_api(const char* method, GVariant* arguments, GError** error);
char** gnoblin_config_runtime_events(void);
void gnoblin_config_finish_event(gboolean commit);
gboolean gnoblin_config_validate_document(GVariant* document, GError** error);

/* Read the selected root. An absent root uses defaults. Errors retain paths
 * and directories so file monitors can retry when a dependency is repaired. */
GVariant* gnoblin_config_load_document(const char* path, GPtrArray** paths, GPtrArray** directories,
                                       GError** error);

/* Expand a config-relative path or glob in deterministic order. `watched_dirs`
 * receives canonical directories that must be monitored for later matches. */
GPtrArray* gnoblin_config_expand_paths(const char* including_file, const char* pattern,
                                       GPtrArray* watched_dirs, GError** error);

/* (Re)load from disk. Safe to call repeatedly. */
void gnoblin_config_reload(void);

/* Last value of `key` in `[section]`. */
gboolean gnoblin_config_get_bool(const char* section, const char* key, gboolean fallback);
int gnoblin_config_get_int(const char* section, const char* key, int fallback);
char* gnoblin_config_get_string(const char* section, const char* key); /* g_free, or NULL */

/* Protocols are available only in the Gnoblin session. Within that boundary,
 * the named [protocols] key defaults on and may explicitly disable a global. */
gboolean gnoblin_config_protocol_enabled(const char* key);

/* All values of a repeated `key` in `[section]`, in file order (e.g. every
 * `exec` in [startup]). g_strfreev, or NULL if none. */
char** gnoblin_config_get_list(const char* section, const char* key);

/* Every key in `[section]`, in file order, including repeated keys (e.g. each
 * accelerator in [bind] or connector in [output]). g_strfreev, or NULL if the
 * section is empty/absent. */
char** gnoblin_config_get_keys(const char* section);

G_END_DECLS
