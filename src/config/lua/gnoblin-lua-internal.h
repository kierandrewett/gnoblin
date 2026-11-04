#ifndef GNOBLIN_LUA_INTERNAL_H
#define GNOBLIN_LUA_INTERNAL_H

/* Restricted, single-state Lua configuration evaluator. */
#include "gnoblin-config.h"
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
#include "../../control/gnoblin-chord.h"
#include "gnoblin-lua-api.h"
#include "gnoblin-lua-pattern.h"
#include "gnoblin-lua-extension.h"
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
#define LUA_GRANT_METATABLE "gnoblin.Grant"
#define LUA_TEXT_TARGET_METATABLE "gnoblin.TextTarget"
#define LUA_WINDOW_DRAG_METATABLE "gnoblin.WindowDrag"
#define LUA_SNAP_CONTEXT_METATABLE "gnoblin.SnapContext"
#define LUA_READONLY_SNAPSHOT_METATABLE "gnoblin.ReadonlySnapshot"
/* The operations one gesture can authorise. A Grant holds the ones it may still take. */
typedef enum {
    LUA_GRANT_FOCUS = 1 << 0,
    LUA_GRANT_BEGIN_MOVE = 1 << 1,
    LUA_GRANT_BEGIN_RESIZE = 1 << 2,
    LUA_GRANT_SNAP_CONTEXT = 1 << 3,
    LUA_GRANT_TEXT_TARGET = 1 << 4,
} LuaGrantScope;
#define LUA_GRANT_ALL                                                                              \
    (LUA_GRANT_FOCUS | LUA_GRANT_BEGIN_MOVE | LUA_GRANT_BEGIN_RESIZE | LUA_GRANT_SNAP_CONTEXT |    \
     LUA_GRANT_TEXT_TARGET)
/* What a Lua script holds as `event.grant`. The compositor made the proof of the gesture; the
 * userdata only names it. What the gesture has spent is in LuaConfig.grant_spent, so every Grant
 * narrowed from one event shares it. */
typedef struct {
    guint64 handle;
    guint64 generation;
    guint64 native_generation;
    gint64 expires_at_us;
    guint scope;   /* LuaGrantScope bits this Grant may take */
    gboolean menu; /* a WM menu request: begin_move and begin_resize on one window, once */
    char window_id[GNOBLIN_GRANT_WINDOW_ID_SIZE]; /* "" until narrowed (a menu starts narrowed) */
} LuaGrant;
typedef struct {
    guint used; /* LuaGrantScope bits already taken */
    gint64 expires_at_us;
} LuaGrantSpent;
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
    char window_id[GNOBLIN_GRANT_WINDOW_ID_SIZE];
} LuaFocusOperation;
typedef struct {
    gsize allocated;
    GPtrArray *paths, *directories;
    GPtrArray* runtime_actions;
    GPtrArray* deferred_callbacks;
    GHashTable* operations;
    GHashTable* timers;    /* Operation ID -> LuaTimer for runtime-local timers. */
    GHashTable* processes; /* process.spawn request ID -> registry reference to the Process. */
    GHashTable* requests;  /* "family:id" -> registry reference to a Request's backing table. */
    GHashTable* focus_operations;
    GHashTable* grant_spent; /* gesture handle (menu bit set for a menu) -> LuaGrantSpent */
    GVariant* window_snapshot;
    guint64 window_revision;
    GVariant* workspace_snapshot;
    guint64 workspace_revision;
    GVariant* monitor_snapshot;
    guint64 monitor_revision;
    GVariant* output_snapshot;
    guint64 output_revision;
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
    GVariant* animation_snapshot;
    guint64 animation_revision;
    GVariant* privacy_snapshot;
    guint64 privacy_revision;
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
    /* The candidate configuration is with the compositor: a playing tween that ends now was
     * ended by the reload (gnoblin_config_reload_in_flight). */
    gboolean reload_in_flight;
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
    gboolean system_dropins;
    GVariant* result;
    GError* error;
} EvalRun;
void console_register_signatures(lua_State* state);
#define MAX_CONSOLE_INSPECTION_DEPTH 8
#define MAX_CONSOLE_INSPECTION_NODES 4000
#define MAX_CONSOLE_INSPECTION_CHILDREN 512
#define MAX_CONSOLE_INSPECTION_TEXT_BYTES (256 * 1024)
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
extern GnoblinConfigSettingsChangedFunc settings_changed_callback;
extern gpointer settings_changed_data;
void push_grant(lua_State* state, guint64 handle, guint64 generation, guint64 native_generation,
                gint64 expires_at_us, gboolean menu, const char* window_id);
gboolean lua_push_window_by_id(lua_State* state, LuaConfig* config, const char* id);
gboolean lua_push_workspace_by_id(lua_State* state, LuaConfig* config, const char* id);
int lua_window_grant_operation(lua_State* state, LuaConfig* config, const char* method,
                               const char* window_id, int grant_index, const char* edge);
gboolean lua_push_workspace_selector(lua_State* state, int index);
gboolean lua_push_monitor_id(lua_State* state, int index);
void push_document(lua_State* state);
LuaConfig* lua_state_config(lua_State* state);
void set_document(lua_State* state, GVariant* document);
gboolean lua_in_runtime(lua_State* state);
void push_configure_operation(lua_State* state, LuaConfig* config);
void finish_configure_operations(lua_State* state, LuaConfig* config, gboolean ok,
                                 const char* message);
int lua_gnoblin_version(lua_State* state);
int lua_config_path(lua_State* state);
int lua_config_default(lua_State* state);
gboolean append_array_key(const char* key);
lua_State* new_config_state(LuaConfig* config);
void limit_hook(lua_State* state, lua_Debug* debug);
void add_path(GPtrArray* paths, const char* path);
char* resolve_path(const char* current, const char* given);
LuaConfig* config_from_upvalue(lua_State* state);
const char* lua_error_text(lua_State* state);
void record_runtime_callback_error(const char* scope, const char* detail);
int handler_pcall(lua_State* state, int arguments, int results);
void handler_outcome(lua_State* state, int function, const char* kind, const char* name,
                     const char* failure);
gboolean call_handler(lua_State* state, int arguments, const char* kind, const char* name);
gboolean marked_array(lua_State* state, int index);
void mark_array_table(lua_State* state, int index);
GVariant* variant_from_lua(lua_State* state, int index, int depth, gboolean force_array,
                           int keybinding_depth, GError** error);
void push_variant(lua_State* state, GVariant* value);
void push_readonly_copy(lua_State* state, int index);
void push_readonly_event_fields(lua_State* state, GVariant* payload, int event_index);
void push_event_table(lua_State* state, const char* event, GVariant* payload);
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
int lua_once(lua_State* state);
gboolean workspace_id_valid(const char* id);
gboolean workspace_selector_id_valid(const char* id);
guint64 allocate_operation_id(void);
void push_operation_handle(lua_State* state, LuaConfig* config, guint64 id, const char* method);
void run_operation_callbacks(lua_State* state, int handle);
GHashTable* runtime_timers_new(void);
int lua_timers_after(lua_State* state);
int lua_timers_now(lua_State* state);
void push_operation_error(lua_State* state, const char* code, const char* message);
void push_local_operation(lua_State* state, LuaConfig* config, const char* method);
void finish_operation(lua_State* state, LuaConfig* config, int handle, gboolean ok, int value,
                      int error);
void subscribe_operation(lua_State* state, LuaConfig* config, int operation);
GVariant* lua_command_argv_variant(lua_State* state, int index, const char* label);
int lua_process_spawn(lua_State* state);
int lua_process_list(lua_State* state);
int lua_process_by_id(lua_State* state);
int lua_process_wait(lua_State* state);
int lua_process_kill(lua_State* state);
void process_exited(lua_State* state, LuaConfig* config, GVariant* payload);
void finish_operation_after(lua_State* state, LuaConfig* config, int handle, double seconds,
                            const char* value);
int request_for_event(lua_State* state, LuaConfig* config, const char* event, GVariant* payload);
GHashTable* runtime_requests_new(void);
void request_end_all(lua_State* state, LuaConfig* config, const char* reason);
gboolean lua_object_kind_is_live(const char* kind);
const char* lua_object_animatable_kind(lua_State* state, int index, const char* property);
char* lua_object_describe_animatable(void);
int lua_object_private(lua_State* state, int handle);
void lua_object_apply(lua_State* state, const char* kind, int record);
void lua_object_raise(lua_State* state, const char* kind, int handle, const char* signal);
void lua_object_push_subscription(lua_State* state, int entry);
void lua_object_update(lua_State* state, const char* kind, int handle, int changes);
void push_readonly_record(lua_State* state, int index, const char* forced_kind);
gboolean lua_api_class_field_observable(const char* class_name, const char* field);
char* lua_api_class_observable_names(const char* class_name);
int lua_find(lua_State* state);
int lua_read_windows(lua_State* state);
int lua_read_layers(lua_State* state);
gboolean lua_object_is(lua_State* state, int index, const char* kind);
int lua_ease(lua_State* state);
int lua_lerp(lua_State* state);
int lua_tween_new(lua_State* state);
int lua_tween_property(lua_State* state);
int lua_tween_interval(lua_State* state);
int lua_tween_callback(lua_State* state);
int lua_tween_parallel(lua_State* state);
int lua_tween_play(lua_State* state);
int lua_tween_pause(lua_State* state);
int lua_tween_seek(lua_State* state);
int lua_tween_kill(lua_State* state);
gboolean lua_tween_dispatch_event(lua_State* state, const char* event, GVariant* payload);
void lua_object_intern(lua_State* state, const char* kind, int index);
void lua_object_hold(lua_State* state, const char* kind, int index);
void lua_object_push_held(lua_State* state, const char* kind);
void lua_object_end(lua_State* state, const char* kind, int handle, const char* reason);
void lua_object_end_all(lua_State* state, const char* kind, const char* reason);
void lua_object_end_after(lua_State* state, LuaConfig* config, int operation, const char* kind,
                          int handle, gboolean end_on_success);
gboolean lua_object_push_registered(lua_State* state, const char* kind, const char* key);
void lua_object_forget(lua_State* state, const char* kind, const char* key);
void lua_object_add_methods(lua_State* state, int method_table, const char* kind);
int lua_object_on(lua_State* state);
int lua_object_watch(lua_State* state);
gboolean lua_object_dispatch_event(lua_State* state, const char* event, GVariant* payload);
gboolean table_fields_allowed(lua_State* state, int index, const char* const* allowed);
int lua_workspace_action(lua_State* state);
int lua_generic_api_action(lua_State* state);
void lua_check_operation_context(lua_State* state, LuaConfig* config);
int lua_queue_operation(lua_State* state, LuaConfig* config, const char* method,
                        GVariant* arguments);
int lua_input_text_target(lua_State* state);
int lua_grant_index(lua_State* state);
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
int lua_focus_policy(lua_State* state);
int lua_windows_list(lua_State* state);
int lua_windows_focused(lua_State* state);
int lua_windows_by_id(lua_State* state);
int lua_workspaces_list(lua_State* state);
int lua_workspaces_active(lua_State* state);
int lua_workspaces_by_id(lua_State* state);
int lua_read_monitors(lua_State* state);
int lua_outputs_list(lua_State* state);
int lua_outputs_primary(lua_State* state);
int lua_outputs_by_id(lua_State* state);
int lua_outputs_renderer(lua_State* state);
int lua_read_privacy_screen(lua_State* state);
int lua_monitors_set_privacy_screen(lua_State* state);
gboolean any_output_has_privacy_screen(LuaConfig* config);
int lua_layers_list(lua_State* state);
int lua_capabilities_list(lua_State* state);
int lua_privacy_state(lua_State* state);
int lua_session_status(lua_State* state);
int lua_runtime_status(lua_State* state);
int lua_input_devices(lua_State* state);
int lua_input_sources(lua_State* state);
int lua_input_current_source(lua_State* state);
int lua_appearance_color_scheme(lua_State* state);
int lua_input_orientation_lock(lua_State* state);
int lua_shortcuts_list(lua_State* state);
int lua_shortcuts_configured(lua_State* state);
int lua_launches_list(lua_State* state);
int lua_launches_snapshot(lua_State* state);
int call_launch_operation(lua_State* state, const char* method, int arguments);
int lua_launches_begin(lua_State* state);
int lua_shortcuts_bind(lua_State* state);
int lua_idle_watch(lua_State* state);
int lua_input_grab(lua_State* state);
int lua_settings_override(lua_State* state);
int lua_window_set_effect(lua_State* state, LuaConfig* config, const char* window_id,
                          int name_index, int value_index);
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
