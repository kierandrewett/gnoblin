#ifndef GNOBLINCTL_INTERNAL_H
#define GNOBLINCTL_INTERNAL_H

/* Gnoblin control client. Uses the libraries already required by the session. */
#include <gio/gio.h>
#include <gio/gunixsocketaddress.h>
#include <glib-unix.h>
#include "gnoblin-config.h"
#include <json-glib/json-glib.h>
#include <glib/gstdio.h>
#include <lauxlib.h>
#include <lua.h>
#include <lualib.h>
#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

typedef struct _CliLuaEventSubscription CliLuaEventSubscription;
typedef struct {
    const char* command;
    const char* action;
    GPtrArray* positionals;
    GHashTable* options;
    const char* format;
    const char* socket_path;
    guint timeout;
    gboolean help;
    gboolean version;
    lua_State* lua_state;
    GPtrArray* lua_event_subscriptions;
    GHashTable* lua_shortcut_bindings;
    GMainLoop* lua_event_loop;
    guint active_lua_event_subscriptions;
    gboolean lua_event_failed;
    CliLuaEventSubscription* lua_shortcut_owner;
} Cli;
struct _CliLuaEventSubscription {
    Cli* cli;
    GSocketConnection* connection;
    GString* pending;
    GHashTable* location_requests;
    char* event_name;
    int callback_ref;
    guint source_id;
    guint pending_source_id;
    gboolean once;
    gboolean once_fired;
    gboolean active;
    gboolean dispatching;
    gboolean shortcut_owner;
    gboolean shortcut_owner_transport;
};
typedef struct {
    gint64 expires_at_us;
    guint requested_accuracy;
} CliLuaLocationRequest;
typedef struct {
    CliLuaEventSubscription* subscription;
} LuaCliEventSubscriptionHandle;
typedef struct {
    guint64 magic;
    CliLuaEventSubscription* subscription;
    char* token;
    gboolean consumed;
} LuaCliFocusContext;
#define LUA_CLI_FOCUS_CONTEXT_MAGIC G_GUINT64_CONSTANT(0x474e4f424c494e46)
typedef struct {
    guint64 magic;
    CliLuaEventSubscription* subscription;
    char* token;
    gboolean consumed;
} LuaCliMenuContext;
#define LUA_CLI_MENU_CONTEXT_MAGIC G_GUINT64_CONSTANT(0x474e4f424d454e55)
typedef struct {
    guint64 magic;
    CliLuaEventSubscription* subscription;
    char* token;
    gboolean consumed;
} LuaCliSnapContext;
#define LUA_CLI_SNAP_CONTEXT_MAGIC G_GUINT64_CONSTANT(0x474e4f42534e4150)
typedef struct {
    guint64 magic;
    CliLuaEventSubscription* subscription;
    char* token;
    gboolean consumed;
} LuaCliTextTarget;
#define LUA_CLI_TEXT_TARGET_MAGIC G_GUINT64_CONSTANT(0x474e4f4254455854)
typedef struct {
    const char* name;
    const char* actions;
} CommandSpec;
typedef struct {
    const char* command;
    const char* action; /* NULL describes the command itself. */
    const char* summary;
} HelpEntry;
#define GNOBLINCTL_READONLY_TABLE_METATABLE "gnoblinctl.ReadonlyTable"
#define GNOBLINCTL_WINDOW_RECORD_METATABLE "gnoblinctl.Window"
#define GNOBLINCTL_WORKSPACE_RECORD_METATABLE "gnoblinctl.Workspace"
#define GNOBLINCTL_MONITOR_RECORD_METATABLE "gnoblinctl.Monitor"
#define GNOBLINCTL_LAYER_SURFACE_RECORD_METATABLE "gnoblinctl.LayerSurface"
#define GNOBLINCTL_ANIMATION_PREVIEW_RECORD_METATABLE "gnoblinctl.AnimationPreview"
#define GNOBLINCTL_PORTAL_GRANT_RECORD_METATABLE "gnoblinctl.PortalGrant"
#define GNOBLINCTL_INPUT_DEVICE_RECORD_METATABLE "gnoblinctl.InputDevice"
#define GNOBLINCTL_INPUT_SOURCE_RECORD_METATABLE "gnoblinctl.InputSource"
#define GNOBLINCTL_SHORTCUT_STATE_RECORD_METATABLE "gnoblinctl.ShortcutState"
#define GNOBLINCTL_SHORTCUT_ACTION_RECORD_METATABLE "gnoblinctl.ShortcutAction"
#define GNOBLINCTL_FOCUS_POLICY_RECORD_METATABLE "gnoblinctl.FocusPolicy"
#define GNOBLINCTL_SETTINGS_RECORD_METATABLE "gnoblinctl.Settings"
#define GNOBLINCTL_LAYER_ANIMATION_POLICY_RECORD_METATABLE "gnoblinctl.LayerAnimationPolicy"
#define GNOBLINCTL_PRIVACY_STATE_RECORD_METATABLE "gnoblinctl.PrivacyState"
#define GNOBLINCTL_CAPABILITY_RECORD_METATABLE "gnoblinctl.Capability"
#define GNOBLINCTL_PERMISSION_POLICY_RECORD_METATABLE "gnoblinctl.PermissionPolicy"
#define GNOBLINCTL_SESSION_STATUS_RECORD_METATABLE "gnoblinctl.SessionStatus"
#define GNOBLINCTL_SESSION_ACTIVITY_RECORD_METATABLE "gnoblinctl.SessionActivity"
#define GNOBLINCTL_PERMISSION_DECISION_RECORD_METATABLE "gnoblinctl.PermissionDecision"
#define GNOBLINCTL_EVENT_SUBSCRIPTION_METATABLE "gnoblinctl.EventSubscription"
#define GNOBLINCTL_FOCUS_CONTEXT_METATABLE "gnoblinctl.FocusContext"
#define GNOBLINCTL_MENU_CONTEXT_METATABLE "gnoblinctl.MenuContext"
#define GNOBLINCTL_SNAP_CONTEXT_METATABLE "gnoblinctl.SnapContext"
#define GNOBLINCTL_TEXT_TARGET_METATABLE "gnoblinctl.TextTarget"

/* Declarations shared between the split files. */
extern const CommandSpec commands[];
extern const gsize commands_count;
const CommandSpec* find_command(const char* name);
gboolean word_in(const char* words, const char* word);
const char* help_summary(const char* command, const char* action);
char* closest_word(const char* typed, const char* words);
const char* option(Cli* cli, const char* name);
gboolean has(Cli* cli, const char* name);
gboolean parse_uint(const char* value, guint low, guint high, guint* result);
gboolean parse_int(const char* value, gint low, gint high, gint* result);
const char* member_string(JsonObject* object, const char* name, const char* fallback);
JsonObject* member_object(JsonObject* object, const char* name);
JsonArray* member_array(JsonObject* object, const char* name);
void print_version(const char* format);
gboolean parse_cli(Cli* cli, int argc, char** argv, GError** error);
guint api_minor_for_method(const char* method);
JsonNode* call_compositor_with_api_minor(Cli* cli, const char* op, const char* method,
                                         JsonObject* arguments, guint api_minor, GError** error);
JsonNode* call_compositor(Cli* cli, const char* op, const char* method, JsonObject* arguments,
                          GError** error);
JsonNode* cli_lua_call_compositor_on_subscription(CliLuaEventSubscription* subscription,
                                                  const char* method, JsonObject* arguments,
                                                  GError** error);
JsonNode* cli_lua_call_compositor_direct_on_subscription(CliLuaEventSubscription* subscription,
                                                         const char* method, JsonObject* arguments,
                                                         GError** error);
char* focused_window_id(Cli* cli, GError** error);
JsonNode* window_match_from_snapshot(JsonArray* windows, const char* selector, GError** error);
char* monitor_id_for_index(Cli* cli, guint monitor_index, GError** error);
gboolean is(const char* left, const char* right);
const char* arg(Cli* cli, guint index);
guint arg_count(Cli* cli);
gboolean validate_cli(Cli* cli, GError** error);
gboolean require_count(Cli* cli, guint minimum, guint maximum, GError** error);
gboolean number_arg(Cli* cli, guint index, const char* name, gint low, gint high, gint* result,
                    GError** error);
JsonNode* string_node(const char* value);
JsonObject* new_object(void);
void set_string(JsonObject* object, const char* name, const char* value);
void set_boolean(JsonObject* object, const char* name, gboolean value);
void set_number(JsonObject* object, const char* name, gint64 value);
void set_if(JsonObject* object, const char* name, const char* value);
gboolean workspace_selector(Cli* cli, guint positional, JsonObject* object, GError** error);
char* config_path(void);
char* default_config(GError** error);
gboolean initialize_config(GError** error);
gboolean restore_default_config(GError** error);
JsonNode* dispatch(Cli* cli, GError** error);
JsonNode* lua_to_json(lua_State* state, int index, guint depth, GError** error);
void json_to_lua(lua_State* state, JsonNode* node);
void lua_cli_push_readonly_value(lua_State* state, int index);
int lua_cli_readonly_index(lua_State* state);
int lua_cli_readonly_newindex(lua_State* state);
void register_lua_cli_readonly_table(lua_State* state);
int lua_cli_window_index(lua_State* state);
int lua_cli_window_newindex(lua_State* state);
int lua_cli_window_len(lua_State* state);
int lua_cli_window_pairs(lua_State* state);
int lua_cli_window_tostring(lua_State* state);
const char* lua_cli_window_id(lua_State* state);
const char* lua_cli_workspace_id(lua_State* state);
void lua_cli_push_animation_preview_record(lua_State* state, Cli* cli, JsonObject* object,
                                           const char* session);
JsonObject* lua_cli_table_object(lua_State* state, int index, const char* description);
int lua_cli_animations_preview(lua_State* state);
int lua_cli_animations_control(lua_State* state);
int lua_cli_animations_list(lua_State* state);
int lua_cli_animations_get(lua_State* state);
int lua_cli_animations_inspect(lua_State* state);
int lua_cli_animations_surfaces(lua_State* state);
int lua_cli_launches_read(lua_State* state);
int lua_cli_launches_operation(lua_State* state);
LuaCliFocusContext* lua_cli_test_focus_context(lua_State* state, int index);
LuaCliMenuContext* lua_cli_test_menu_context(lua_State* state, int index);
int lua_cli_menu_context_begin_move(lua_State* state);
int lua_cli_menu_context_begin_resize(lua_State* state);
void lua_cli_push_window_record(lua_State* state, Cli* cli, JsonObject* object);
int lua_cli_snap_context_gc(lua_State* state);
int lua_cli_snap_context_newindex(lua_State* state);
int lua_cli_snap_context_tostring(lua_State* state);
void lua_cli_push_snap_context(lua_State* state, CliLuaEventSubscription* subscription,
                               JsonObject* response);
int lua_cli_text_target_gc(lua_State* state);
int lua_cli_text_target_newindex(lua_State* state);
int lua_cli_text_target_tostring(lua_State* state);
int lua_cli_input_text_target(lua_State* state);
int lua_cli_workspace_tostring(lua_State* state);
void lua_cli_push_workspace_record(lua_State* state, Cli* cli, JsonObject* object);
int lua_cli_monitor_tostring(lua_State* state);
void lua_cli_push_monitor_record(lua_State* state, JsonObject* object);
int lua_cli_layer_surface_tostring(lua_State* state);
void lua_cli_push_layer_surface_record(lua_State* state, JsonObject* object);
void lua_cli_push_input_record(lua_State* state, JsonObject* object, gint64 revision,
                               gboolean is_device);
int lua_cli_shortcuts_list(lua_State* state);
int lua_cli_shortcuts_actions(lua_State* state);
int lua_cli_shortcuts_capture(lua_State* state);
int lua_cli_shortcuts_bind(lua_State* state);
int lua_cli_shortcuts_unbind(lua_State* state);
int lua_cli_shortcuts_end_session(lua_State* state);
void register_lua_cli_shortcut_record(lua_State* state, const char* metatable);
gboolean lua_cli_focus_policy_valid(JsonObject* object);
void register_lua_cli_focus_policy_record(lua_State* state);
void register_lua_cli_settings_record(lua_State* state);
int lua_cli_settings_property(lua_State* state);
void register_lua_cli_layer_animation_policy_record(lua_State* state);
int lua_cli_layer_animation_policy(lua_State* state);
void register_lua_cli_privacy_state_record(lua_State* state);
int lua_cli_privacy_state(lua_State* state);
int lua_cli_privacy_stop(lua_State* state);
void cli_lua_location_request_track(CliLuaEventSubscription* subscription, JsonObject* event);
int lua_cli_location_authorize_app(lua_State* state);
int lua_cli_runtime_reload_config(lua_State* state);
int lua_cli_runtime_status(lua_State* state);
int lua_cli_session_operation(lua_State* state);
void register_lua_cli_capability_record(lua_State* state);
int lua_cli_capabilities_list(lua_State* state);
void register_lua_cli_permission_policy_record(lua_State* state);
int lua_cli_permissions_list(lua_State* state);
int lua_cli_version(lua_State* state);
int lua_cli_permissions_policy(lua_State* state);
void register_lua_cli_permission_decision_record(lua_State* state);
int lua_cli_permissions_check(lua_State* state);
void register_lua_cli_session_status_record(lua_State* state);
int lua_cli_session_status(lua_State* state);
void register_lua_cli_session_activity_record(lua_State* state);
int lua_cli_session_activity(lua_State* state);
int lua_cli_focus_policy_property(lua_State* state);
int lua_cli_input_snapshot(lua_State* state);
int lua_cli_appearance_color_scheme(lua_State* state);
int lua_cli_input_select_source(lua_State* state);
int lua_cli_input_set_orientation_lock(lua_State* state);
int lua_cli_grant_revoke(lua_State* state);
int lua_cli_portal_grants(lua_State* state);
int lua_cli_layers_list(lua_State* state);
void register_lua_cli_layer_surface_record(lua_State* state);
void register_lua_cli_animation_preview_record(lua_State* state);
void register_lua_cli_portal_grant_record(lua_State* state);
void register_lua_cli_input_record(lua_State* state, const char* metatable);
int lua_cli_monitors_list(lua_State* state);
int lua_cli_monitors_primary(lua_State* state);
int lua_cli_monitors_privacy_screen(lua_State* state);
int lua_cli_monitors_set_privacy_screen(lua_State* state);
int lua_cli_workspaces_list(lua_State* state);
int lua_cli_workspaces_active(lua_State* state);
int lua_cli_workspaces_by_id(lua_State* state);
gboolean lua_cli_workspace_object_has_only_keys(JsonObject* object, const char* const* allowed);
int lua_cli_workspaces_operation(lua_State* state);
int lua_cli_windows_list(lua_State* state);
int lua_cli_windows_snap_context(lua_State* state);
int lua_cli_focus_history(lua_State* state);
int lua_cli_windows_focused(lua_State* state);
int lua_cli_windows_by_id(lua_State* state);
void cli_lua_event_subscription_detach(CliLuaEventSubscription* subscription);
gboolean cli_lua_shortcut_owner_dispatch(CliLuaEventSubscription* owner, JsonNode* event);
void cli_lua_event_subscription_free(gpointer data);
void lua_cli_push_focus_context(lua_State* state, CliLuaEventSubscription* subscription,
                                const char* token);
void lua_cli_push_menu_context(lua_State* state, CliLuaEventSubscription* subscription,
                               const char* token);
gboolean cli_lua_event_dispatch(CliLuaEventSubscription* subscription, JsonNode* event);
gboolean cli_lua_shortcut_owner_ensure(Cli* cli, GError** error);
int lua_cli_events_subscribe(lua_State* state);
void register_lua_cli_focus_context(lua_State* state);
void register_lua_cli_menu_context(lua_State* state);
void register_lua_cli_snap_context(lua_State* state);
void register_lua_cli_text_target(lua_State* state);
void register_lua_cli_event_subscription(lua_State* state);
gboolean cli_lua_event_loop(Cli* cli);
void cli_lua_events_clear(Cli* cli);
int lua_api_index(lua_State* state);
void register_lua_cli_window_record(lua_State* state);
void register_lua_cli_workspace_record(lua_State* state);
void register_lua_cli_monitor_record(lua_State* state);
void print_lua_error(lua_State* state, const char* prefix);
int run_lua_console(Cli* cli, const char* file);
void render(const char* command, const char* action, JsonNode* result, const char* format,
            gboolean raw_string);
/* Terminal rendering helpers shared by the command renderers. */
char* node_text(JsonNode* node);
char* human_label(const char* name);
void print_table(JsonArray* array);
void print_key_values(JsonObject* object, const char* const* keys, const char* const* labels,
                      guint count);
void print_key_value(JsonObject* object, const char* key, const char* label);
/* Print a command result for a terminal. Returns FALSE to use the generic renderer. */
gboolean render_command(const char* command, const char* action, JsonNode* result);
gboolean render_spatial(const char* command, const char* action, JsonNode* result);
gboolean render_state(const char* command, const char* action, JsonNode* result);
gboolean render_settings(const char* command, const char* action, JsonNode* result);
void print_help(const char* command, const char* action);
void print_completion(const char* shell);

#endif /* GNOBLINCTL_INTERNAL_H */

int lua_cli_outputs_list(lua_State* state);
int lua_cli_outputs_primary(lua_State* state);
int lua_cli_outputs_by_id(lua_State* state);
int lua_cli_outputs_renderer(lua_State* state);
