#ifndef GNOBLIN_LUA_INTERNAL_H
#define GNOBLIN_LUA_INTERNAL_H

/* Restricted, single-state Lua configuration evaluator. */
#include "gnoblin-config.h"
#include "gnoblin-portal-policy.h"
#include <gio/gio.h>
#include <glib/gstdio.h>
#include <json-glib/json-glib.h>
#include <fnmatch.h>
#include <errno.h>
#include <lauxlib.h>
#include <lua.h>
#include <lualib.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>
#include "gnoblin-keybinding-catalog.h"
#include "gnoblin-lua-pattern.h"
#if __has_include("../core/gnoblin-native-control.h")
#include "../core/gnoblin-native-control.h"
#endif
#define MAX_CONFIG_BYTES (8 * 1024 * 1024)
#define MAX_CONFIG_STEPS 1000000
#define MAX_CONFIG_DEPTH 64
#define MAX_CONFIG_FILES 32
#define MAX_CONSOLE_SOURCE_BYTES (16 * 1024)
#define MAX_CONSOLE_RESULT_BYTES (16 * 1024)
#define MAX_CONSOLE_STEPS 100000
#define LUA_FOCUS_CONTEXT_METATABLE "gnoblin.FocusContext"
#define LUA_MENU_CONTEXT_METATABLE "gnoblin.MenuContext"
#define LUA_TEXT_TARGET_METATABLE "gnoblin.TextTarget"
#define LUA_WINDOW_DRAG_METATABLE "gnoblin.WindowDrag"
#define LUA_SNAP_CONTEXT_METATABLE "gnoblin.SnapContext"
#define LUA_READONLY_SNAPSHOT_METATABLE "gnoblin.ReadonlySnapshot"
typedef struct {
    guint64 handle;
    guint64 generation;
    guint64 native_generation;
    gint64 expires_at_us;
    gboolean consumed;
} LuaFocusContext;
typedef struct {
    guint64 handle;
    guint64 generation;
    guint64 native_generation;
    gint64 expires_at_us;
    char window_id[24];
    gboolean consumed;
} LuaMenuContext;
typedef struct {
    char token[65];
    char window_id[24];
    guint64 generation;
    gboolean consumed;
    gboolean has_caret;
    double x, y, width, height;
} LuaTextTarget;
typedef struct {
    guint64 id;
    guint64 settings_revision;
    guint64 generation;
    gboolean ended;
} LuaWindowDrag;
typedef struct {
    char token[65];
    guint64 generation;
    gboolean consumed;
} LuaSnapContext;
typedef struct {
    guint64 handle;
    guint64 generation;
    guint64 native_generation;
    gint64 expires_at_us;
} LuaFocusOperation;
typedef struct {
    gsize allocated;
    GPtrArray *paths, *directories;
    GPtrArray* runtime_actions;
    GPtrArray* deferred_callbacks;
    GHashTable* operations;
    GHashTable* focus_operations;
    GVariant* window_snapshot;
    guint64 window_revision;
    GVariant* workspace_snapshot;
    guint64 workspace_revision;
    GVariant* monitor_snapshot;
    guint64 monitor_revision;
    GVariant* monitor_privacy_screen_snapshot;
    GVariant* layer_snapshot;
    guint64 layer_revision;
    GVariant* capability_snapshot;
    guint64 capability_revision;
    GVariant* input_device_snapshot;
    guint64 input_device_revision;
    GVariant* input_source_snapshot;
    guint64 input_source_revision;
    GVariant* orientation_lock_snapshot;
    guint64 orientation_lock_revision;
    GVariant* appearance_snapshot;
    gboolean appearance_snapshot_received;
    GVariant* shortcut_snapshot;
    guint64 shortcut_revision;
    GVariant* launch_snapshot;
    guint64 launch_revision;
    GVariant* portal_grant_snapshot;
    guint64 portal_grant_revision;
    GVariant* animation_snapshot;
    guint64 animation_revision;
    GVariant* privacy_snapshot;
    guint64 privacy_revision;
    GVariant* session_activity_snapshot;
    guint64 session_activity_revision;
    gint64 session_activity_sampled_at_us;
    GVariant* session_lock_snapshot;
    guint64 session_lock_revision;
    GVariant* session_lifecycle_snapshot;
    guint64 session_lifecycle_revision;
    guint64 input_gesture_sequence;
    guint64 dispatch_focus_handle;
    guint64 dispatch_focus_generation;
    guint64 dispatch_native_focus_generation;
    gint64 dispatch_focus_expires_at_us;
    guint64 dispatch_menu_handle;
    guint64 dispatch_menu_generation;
    guint64 dispatch_native_menu_generation;
    gint64 dispatch_menu_expires_at_us;
    char dispatch_menu_window_id[24];
    GVariant* settings_document;
    guint64 settings_revision;
    GPtrArray* focus_history_ids;
    GHashTable *active, *modules;
    char* current_path;
    guint next_input_handler_id;
    guint actions_in_dispatch;
    gboolean dispatching;
    gboolean input_dispatching;
    gboolean api_calling;
    gboolean ignore_config_loads;
    /* Login: an included file that fails is skipped and reported instead of ending the whole load.
     */
    gboolean skip_failed_files;
    GPtrArray* skipped_files;
    gboolean embedded_defaults;
    gboolean rejecting_operations;
    gboolean deferred_callbacks_scheduled;
} LuaConfig;
typedef struct {
    LuaConfig config;
    lua_State* state;
    GVariant* document;
    GVariant* pending_document;
    guint64 generation;
} LuaRuntime;
typedef struct {
    char* gnoblin;
    char* gnome;
    char* mutter;
    char* git_remote;
    char* git_sha;
    char* build_id;
} VersionIdentity;
typedef struct {
    LuaConfig* config;
    const char* path;
    const char* source;
    gsize source_length;
    GVariant* result;
    GError* error;
} EvalRun;
#define MAX_CONSOLE_INSPECTION_DEPTH 6
#define MAX_CONSOLE_INSPECTION_NODES 200
#define MAX_CONSOLE_INSPECTION_CHILDREN 64
#define MAX_CONSOLE_INSPECTION_VALUE_BYTES 1024
typedef struct {
    guint nodes;
    gsize text_bytes;
    GHashTable* seen;
} ConsoleInspection;
typedef struct {
    char* label;
    char* detail;
    char* documentation;
} ConsoleCompletion;
#define MAX_CONSOLE_COMPLETIONS 64
#define MAX_CONSOLE_COMPLETION_SEGMENTS 16
#define MAX_CONSOLE_COMPLETION_TOKEN_BYTES 1024
#define MAX_CONSOLE_COMPLETION_TABLE_VISITS 512

/* Declarations shared between the split files. */
extern LuaRuntime* active_runtime;
extern LuaRuntime* pending_runtime;
extern LuaRuntime* deferred_runtime;
extern guint64 next_runtime_operation_id;
extern guint64 next_settings_revision;
extern guint64 next_runtime_generation;
extern gboolean revision_counters_seeded;
extern GnoblinConfigRuntimeWakeupFunc runtime_wakeup_callback;
extern gpointer runtime_wakeup_data;
extern GnoblinConfigFocusPolicyChangedFunc focus_policy_changed_callback;
extern gpointer focus_policy_changed_data;
extern GnoblinConfigPermissionPolicyChangedFunc permission_policy_changed_callback;
extern gpointer permission_policy_changed_data;
extern GnoblinConfigSettingsChangedFunc settings_changed_callback;
extern gpointer settings_changed_data;
void push_menu_context(lua_State* state, guint64 handle, guint64 generation,
                       guint64 native_generation, gint64 expires_at_us, const char* window_id);
void push_focus_context(lua_State* state, guint64 handle, guint64 generation,
                        guint64 native_generation, gint64 expires_at_us);
extern const char* api_methods[];
typedef struct {
    const char* public_name;
    const char* operation;
} ApiAlias;

extern const ApiAlias api_aliases[];
extern const gsize api_aliases_count;
gboolean append_array_key(const char* key);
lua_State* new_config_state(LuaConfig* config);
void limit_hook(lua_State* state, lua_Debug* debug);
void add_path(GPtrArray* paths, const char* path);
char* resolve_path(const char* current, const char* given);
LuaConfig* config_from_upvalue(lua_State* state);
const char* lua_error_text(lua_State* state);
void record_runtime_callback_error(const char* scope, const char* detail);
gboolean marked_array(lua_State* state, int index);
void mark_array_table(lua_State* state, int index);
GVariant* variant_from_lua(lua_State* state, int index, int depth, gboolean force_array,
                           int keybinding_depth, GError** error);
void push_variant(lua_State* state, GVariant* value);
void push_readonly_copy(lua_State* state, int index);
void push_readonly_event_fields(lua_State* state, GVariant* payload, int event_index);
void push_window_drag(lua_State* state, GVariant* value);
void push_operation_result(lua_State* state, const char* method, GVariant* value);
void remove_named_entry(lua_State* state, int list, const char* name);
int compare_entry_names(gconstpointer left, gconstpointer right);
void merge_named_entries(lua_State* state, int list, int entries);
void merge_table(lua_State* state, int destination, int source, const char* key,
                 gboolean append_lists);
int lua_array(lua_State* state);
void copy_config_value(lua_State* state, int source, int depth);
void push_public_settings(lua_State* state, GVariant* value, const char* parent, int depth,
                          int keybinding_depth);
int lua_gnoblin_index(lua_State* state);
int lua_snapshot(lua_State* state);
int lua_set(lua_State* state);
int lua_on(lua_State* state);
int lua_mutter_on(lua_State* state);
int lua_once(lua_State* state);
int lua_mutter_once(lua_State* state);
gboolean workspace_id_valid(const char* id);
gboolean workspace_selector_id_valid(const char* id);
guint64 allocate_operation_id(void);
void push_operation_handle(lua_State* state, LuaConfig* config, guint64 id, const char* method);
gboolean table_fields_allowed(lua_State* state, int index, const char* const* allowed);
int lua_workspace_action(lua_State* state);
int lua_generic_api_action(lua_State* state);
int lua_input_text_target(lua_State* state);
int lua_menu_context_begin_move(lua_State* state);
int lua_menu_context_begin_resize(lua_State* state);
int lua_windows_snap_context(lua_State* state);
int lua_snap_context_commit(lua_State* state);
gboolean lua_dense_array(lua_State* state, int index, lua_Integer maximum);
int lua_window_drag_offer_targets(lua_State* state);
int lua_text_target_index(lua_State* state);
int lua_animation_preview_method(lua_State* state);
int lua_layers_animation_policy(lua_State* state);
int lua_animations_list(lua_State* state);
int lua_animation_get(lua_State* state);
void push_settings(lua_State* state, int source, const char* parent, int depth,
                   int keybinding_depth);
int lua_configure_call(lua_State* state);
int lua_input_on(lua_State* state);
void push_config_list(lua_State* state);
void install_named_view(lua_State* state, const char* key);
void finish_named_entries(lua_State* state, int config, const char* key);
int lua_declare(lua_State* state);
int lua_config_load(lua_State* state);
int lua_require(lua_State* state);
void focus_history_reconcile_snapshot(LuaConfig* config, GVariant* snapshot);
void focus_history_update_event(LuaConfig* config, const char* event, GVariant* payload);
int lua_focus_history(lua_State* state);
int lua_windows_list(lua_State* state);
int lua_windows_focused(lua_State* state);
int lua_windows_by_id(lua_State* state);
int lua_workspaces_list(lua_State* state);
int lua_workspaces_active(lua_State* state);
int lua_workspaces_by_id(lua_State* state);
int lua_monitors_list(lua_State* state);
int lua_monitors_primary(lua_State* state);
int lua_monitors_privacy_screen(lua_State* state);
int lua_monitors_set_privacy_screen(lua_State* state);
int lua_layers_list(lua_State* state);
int lua_capabilities_list(lua_State* state);
int lua_permissions_list(lua_State* state);
int lua_permissions_policy(lua_State* state);
int lua_privacy_state(lua_State* state);
int lua_session_activity(lua_State* state);
int lua_session_status(lua_State* state);
int lua_runtime_status(lua_State* state);
int lua_permissions_check(lua_State* state);
int lua_input_devices(lua_State* state);
int lua_input_sources(lua_State* state);
int lua_input_current_source(lua_State* state);
int lua_appearance_color_scheme(lua_State* state);
int lua_input_orientation_lock(lua_State* state);
int lua_shortcuts_list(lua_State* state);
int lua_launches_list(lua_State* state);
int lua_launches_snapshot(lua_State* state);
int lua_portal_grants(lua_State* state);
int call_launch_operation(lua_State* state, const char* method, int arguments);
int lua_launches_begin(lua_State* state);
int lua_launches_finish(lua_State* state);
int lua_shortcut_actions(lua_State* state);
gboolean evaluate_path(lua_State* state, LuaConfig* config, const char* given, gboolean root,
                       GError** error);
gboolean evaluate_embedded_path(lua_State* state, LuaConfig* config, const char* relative,
                                GError** error);
int protected_eval(lua_State* state);
void lua_runtime_free(LuaRuntime* runtime);
void fail_dropped_operation(LuaRuntime* runtime, GVariant* operation, const char* reason);
void schedule_deferred_callbacks(LuaConfig* config);

#endif /* GNOBLIN_LUA_INTERNAL_H */
