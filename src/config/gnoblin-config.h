/*
 * gnoblin: Lua configuration loading for the Gnoblin supervisor.
 *
 * Read from $GNOBLIN_CONFIG, else init.lua under $XDG_CONFIG_HOME/gnoblin.
 * Existing TOML and CONF filenames are detected so they can produce a
 * migration error, but config files and includes must use Lua and the .lua
 * suffix.
 *
 * The supervisor owns the persistent Lua state and evaluates configuration.
 * It transfers immutable native snapshots to Mutter. Missing .lua roots use
 * defaults; invalid reloads retain the last valid runtime.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

/* Immutable shipped default-config tree, embedded in the executable. */
typedef struct {
    const char* path; /* relative to the default-config root */
    const char* contents;
    gsize length;
} GnoblinConfigDefaultFile;

const GnoblinConfigDefaultFile* gnoblin_config_default_files(gsize* count);
const GnoblinConfigDefaultFile* gnoblin_config_default_file(const char* path);
/* Compatibility accessor for the embedded init.lua entry point. */
const char* gnoblin_config_default_lua(gsize* length);
/* Current root filename. Free the result with g_free(). */
char* gnoblin_config_path(void);
/* A validated declarative configuration snapshot for startup recovery. */
GVariant* gnoblin_config_default_document(void);
GVariant* gnoblin_config_load_last_good_document(const char* config_path, GError** error);
gboolean gnoblin_config_save_last_good_document(const char* config_path, GVariant* document,
                                                GError** error);
/* Evaluate one .lua config with a fresh Lua state and record every dependency. */
GVariant* gnoblin_config_evaluate_file(const char* path, GPtrArray* paths, GPtrArray* directories,
                                       GError** error);
/* Persistent Lua runtime owned by the supervisor for config and runtime events. */
GVariant* gnoblin_config_load_runtime(const char* path, GPtrArray** paths, GPtrArray** directories,
                                      GError** error);
/* Start an empty Lua callback state with a previously validated document. */
/* Like gnoblin_config_load_runtime, but a document that fails validation is repaired with
 * gnoblin_config_salvage_document instead of rejected. `ignored` is filled as there. */
GVariant* gnoblin_config_load_runtime_salvaged(const char* path, GPtrArray** paths,
                                               GPtrArray** directories, GPtrArray** ignored,
                                               GError** error);
GVariant* gnoblin_config_load_runtime_fallback(const char* path, GVariant* document,
                                               GPtrArray** paths, GPtrArray** directories,
                                               GError** error);
/* Restore an exact native-accepted document using embedded callbacks. Only use
 * while recovery gates event dispatch until a fresh transactional CONFIG commits. */
GVariant* gnoblin_config_load_runtime_recovery_snapshot(const char* path, GVariant* document,
                                                        GPtrArray** paths, GPtrArray** directories,
                                                        GError** error);
/* Start a live Lua runtime from the embedded default-config tree. Unlike a
 * serialized document fallback, this retains default shortcut callbacks. */
GVariant* gnoblin_config_load_runtime_defaults(GPtrArray** paths, GPtrArray** directories,
                                               GError** error);
void gnoblin_config_finish_load(gboolean commit);
/* Temporarily detach a validated candidate so operations can complete on the
 * current runtime before the candidate replaces it. */
gboolean gnoblin_config_defer_load_commit(void);
void gnoblin_config_finish_deferred_load(gboolean commit);
guint64 gnoblin_config_deferred_runtime_generation(void);
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
/* Cache the monitor privacy-screen snapshot used by immediate Lua reads. */
void gnoblin_config_update_monitor_privacy_screen_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the latest native layer-surface snapshot for immediate Lua reads. */
void gnoblin_config_update_layer_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the latest native capability snapshot for immediate Lua reads. */
void gnoblin_config_update_capability_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the latest native input-device snapshot for immediate Lua reads. */
void gnoblin_config_update_input_device_snapshot(GVariant* snapshot, guint64 revision);
/* Cache configured XKB input sources and the confirmed current source. */
void gnoblin_config_update_input_source_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the live orientation-lock state for Lua reads. */
void gnoblin_config_update_orientation_lock_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the current desktop appearance preference for Lua reads. */
void gnoblin_config_update_appearance_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the configured native shortcut snapshot. */
void gnoblin_config_update_shortcut_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the latest native launch-feedback snapshot for immediate Lua reads. */
void gnoblin_config_update_launch_snapshot(GVariant* snapshot, guint64 revision);
/* Cache validated portal grants for immediate Lua collection reads. */
void gnoblin_config_update_portal_grant_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the current native animation registry for immediate Lua reads. */
void gnoblin_config_update_animation_snapshot(GVariant* snapshot, guint64 revision);
/* Cache native privacy availability and activity fields for immediate Lua reads. */
void gnoblin_config_update_privacy_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the current session activity state for immediate Lua reads. */
void gnoblin_config_update_session_activity_snapshot(GVariant* snapshot, guint64 revision);
/* Cache the compositor's current lock availability and state for status reads. */
void gnoblin_config_update_session_lock_snapshot(GVariant* snapshot, guint64 revision);
void gnoblin_config_update_session_lifecycle_snapshot(GVariant* snapshot, guint64 revision);
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
/* Notify the native host after any committed configuration change. */
typedef void (*GnoblinConfigSettingsChangedFunc)(guint64 revision, gpointer user_data);
void gnoblin_config_set_settings_changed_callback(GnoblinConfigSettingsChangedFunc callback,
                                                  gpointer user_data);
GVariant* gnoblin_config_dispatch_event(const char* event, GVariant* payload, GError** error);

/* Returns and clears the latest contained Lua callback failure. Event-time
 * callbacks are isolated from the session: their subscription is disabled and
 * the supervisor publishes this diagnostic to the recovery UI. */
char* gnoblin_config_take_runtime_callback_error(void);
/* Evaluate a compositor-matched input policy callback. The returned decision
 * is independent of configuration commits; callers drain queued operations
 * only after delivering the decision. */
GVariant* gnoblin_config_dispatch_input(GVariant* payload, GError** error);
/* Dispatch a trusted native shortcut event with a private, userdata-only
 * focus context. The context is never added to the public event payload. */
GVariant* gnoblin_config_dispatch_shortcut_event(const char* event, GVariant* payload,
                                                 guint64 context_handle, guint64 native_generation,
                                                 gint64 expires_at_us, GError** error);
guint64 gnoblin_config_runtime_generation(void);
/* Seed process-local counters from Mutter before the first runtime load.
 * Used only by a replacement runtime worker. */
gboolean gnoblin_config_seed_runtime_counters(guint64 settings_revision, guint64 runtime_generation,
                                              guint64 operation_id_watermark, GError** error);
gboolean gnoblin_config_take_focus_context(gint64 request_id, guint64* context_handle,
                                           guint64* generation, guint64* native_generation,
                                           gint64* expires_at_us);
/* The compositor host schedules deferred Lua callbacks on its main loop and
 * dispatches them through the normal config validation/operation path. */
typedef void (*GnoblinConfigRuntimeWakeupFunc)(gpointer user_data);
void gnoblin_config_set_runtime_wakeup_callback(GnoblinConfigRuntimeWakeupFunc callback,
                                                gpointer user_data);
GVariant* gnoblin_config_dispatch_deferred_callbacks(GError** error);
/* Deferred runtime operations use a shared method registry. Event callbacks
 * enqueue operations; the host drains them only after dispatch returns. */
GVariant* gnoblin_config_drain_runtime_operations(void); /* aa{sv} */
/* Operations and deferred completions that still belong to the active runtime. */
guint gnoblin_config_runtime_pending_operations(void);
GVariant* gnoblin_config_call_api(const char* method, GVariant* arguments, GError** error);
/* Run one bounded Lua console chunk against the active configuration runtime.
 * The result is a vardict containing a display-safe value, its Lua type, and
 * the number of native operations the chunk queued. Console chunks may use
 * the public Gnoblin Lua API, but may not replace the declarative
 * gnoblin.config document. */
GVariant* gnoblin_config_eval_console(const char* source, GError** error);
/* Complete a raw Lua identifier or dot chain without evaluating source. */
GVariant* gnoblin_config_complete_console(const char* source, GError** error);
/* Evaluate one allowlisted read-only Lua snapshot API method. */
GVariant* gnoblin_config_read_api(const char* method, GVariant* arguments, GError** error);
char** gnoblin_config_runtime_events(void);
void gnoblin_config_finish_event(gboolean commit);
gboolean gnoblin_config_validate_document(GVariant* document, GError** error);
/* Return `document` with every invalid key or list entry removed, so the valid settings still apply. `ignored`
 * receives one message per removed item ("ignored window-management.focus-mode: <reason>") and is NULL when
 * nothing was removed. Returns NULL with `error` set when the document cannot be repaired. */
GVariant* gnoblin_config_salvage_document(GVariant* document, GPtrArray** ignored, GError** error);
/* Project gnoblin.configure.portals into the generated XDG portal preferences.
 * `config_home` is NULL to use XDG_CONFIG_HOME. The document must be valid. */
gboolean gnoblin_config_sync_portal_selection(GVariant* document, const char* config_home,
                                              GError** error);
/* Match a value with Lua string.find pattern semantics. Invalid patterns set
 * an error; valid patterns search anywhere unless anchored with ^ or $. */
gboolean gnoblin_config_window_pattern_match(const char* pattern, const char* value,
                                             GError** error);
/* Return the normalized input table with "inherit" leaves and empty groups
 * removed. Missing input returns NULL without setting error. The caller owns
 * the returned reference; invalid input returns NULL and sets error. */
GVariant* gnoblin_config_normalize_input(GVariant* document, GError** error);

/* Read the selected root. An absent root uses defaults. Errors retain paths
 * and directories so file monitors can retry when a dependency is repaired. */
GVariant* gnoblin_config_load_document(const char* path, GPtrArray** paths, GPtrArray** directories,
                                       GError** error);

/* Expand a config-relative path or glob in deterministic order. `watched_dirs`
 * receives canonical directories that must be monitored for later matches. */
GPtrArray* gnoblin_config_expand_paths(const char* including_file, const char* pattern,
                                       GPtrArray* watched_dirs, GError** error);

G_END_DECLS
