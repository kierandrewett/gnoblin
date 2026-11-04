/* session.suspend, session.hibernate, session.reboot and session.poweroff.
 *
 * The compositor asks logind itself and completes the Operation with the answer, so the Lua
 * runtime reports what logind did: success, a refusal (policy or polkit), or the D-Bus error. The
 * runtime stays alive; for a reboot or power off logind ends the session on its own. Each call
 * first reads the capability (CanSuspend and its siblings): "na" is unsupported and "no" is
 * denied. "yes" and "challenge" go on to the action, which asks for interactive authorization. */
#include "config.h"

#include "core/gnoblin-control-internal.h"

#include <gio/gio.h>
#include <json-glib/json-glib.h>

#define LOGIND_NAME "org.freedesktop.login1"
#define LOGIND_PATH "/org/freedesktop/login1"
#define LOGIND_MANAGER "org.freedesktop.login1.Manager"
#define POWER_CAPABILITY_TIMEOUT_MS 5000
#define POWER_ACTION_TIMEOUT_MS 30000

typedef struct {
    GnoblinNativeControl* control;
    gint64 operation_id;
    const char* method;
    const char* capability;
    const char* action;
    const char* label;
    GDBusConnection* bus;
} PowerCall;

static const struct {
    const char* method;
    const char* capability;
    const char* action;
    const char* label;
} power_actions[] = {
    {"session.suspend", "CanSuspend", "Suspend", "suspend"},
    {"session.hibernate", "CanHibernate", "Hibernate", "hibernate"},
    {"session.reboot", "CanReboot", "Reboot", "restart"},
    {"session.poweroff", "CanPowerOff", "PowerOff", "power off"},
};

static void power_call_free(PowerCall* call) {
    g_clear_object(&call->bus);
    g_free(call);
}

static void power_finish(PowerCall* call, const GError* error) {
    GnoblinNativeControl* control = call->control;
    if (control->pending_power_calls)
        control->pending_power_calls--;
    if (!control->stopping) {
        g_autoptr(JsonNode) result = NULL;
        if (!error) {
            JsonObject* object = json_object_new();
            json_object_set_boolean_member(object, "accepted", TRUE);
            result = json_node_new(JSON_NODE_OBJECT);
            json_node_take_object(result, object);
        }
        gnoblin_control_dispatch_operation_completion_full(
            control, call->operation_id, call->method, result != NULL, result,
            gnoblin_control_operation_error_code(error), error ? error->message : NULL, FALSE);
    }
    gnoblin_control_maybe_free_stopped(control);
    power_call_free(call);
}

/* Turn a failed logind call into one error: a code the Lua caller can check, and a message that
 * names the D-Bus error. */
static GError* power_error(const PowerCall* call, const GError* source) {
    g_autofree char* remote = g_dbus_error_get_remote_error(source);
    g_autofree char* encoded = remote ? NULL : g_dbus_error_encode_gerror(source);
    const char* name = remote ? remote : encoded;
    g_autoptr(GError) plain = g_error_copy(source);
    g_dbus_error_strip_remote_error(plain);

    GIOErrorEnum code = G_IO_ERROR_FAILED;
    if (g_error_matches(source, G_IO_ERROR, G_IO_ERROR_TIMED_OUT) ||
        g_error_matches(source, G_DBUS_ERROR, G_DBUS_ERROR_TIMED_OUT) ||
        g_error_matches(source, G_DBUS_ERROR, G_DBUS_ERROR_NO_REPLY))
        code = G_IO_ERROR_TIMED_OUT;
    else if (g_error_matches(source, G_DBUS_ERROR, G_DBUS_ERROR_ACCESS_DENIED) ||
             g_error_matches(source, G_DBUS_ERROR, G_DBUS_ERROR_AUTH_FAILED) ||
             (name &&
              g_str_equal(name, "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired")))
        code = G_IO_ERROR_PERMISSION_DENIED;
    else if (g_error_matches(source, G_DBUS_ERROR, G_DBUS_ERROR_SERVICE_UNKNOWN) ||
             g_error_matches(source, G_DBUS_ERROR, G_DBUS_ERROR_NAME_HAS_NO_OWNER) ||
             g_error_matches(source, G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED))
        code = G_IO_ERROR_NOT_CONNECTED;
    else if (g_error_matches(source, G_DBUS_ERROR, G_DBUS_ERROR_UNKNOWN_METHOD) ||
             g_error_matches(source, G_DBUS_ERROR, G_DBUS_ERROR_NOT_SUPPORTED) ||
             (name && g_str_equal(name, "org.freedesktop.login1.SleepVerbNotSupported")))
        code = G_IO_ERROR_NOT_SUPPORTED;
    else if (name && g_str_equal(name, "org.freedesktop.login1.OperationInProgress"))
        code = G_IO_ERROR_BUSY;
    return g_error_new(G_IO_ERROR, code, "logind could not %s: %s%s%s", call->label,
                       name ? name : "", name ? ": " : "", plain->message);
}

static void power_fail(PowerCall* call, const GError* source) {
    g_autoptr(GError) error = power_error(call, source);
    power_finish(call, error);
}

static void power_action_done(GObject* source, GAsyncResult* async_result, gpointer user_data) {
    PowerCall* call = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source), async_result, &error);
    if (!reply)
        power_fail(call, error);
    else
        power_finish(call, NULL);
}

static void power_capability_done(GObject* source, GAsyncResult* async_result, gpointer user_data) {
    PowerCall* call = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source), async_result, &error);
    if (!reply) {
        power_fail(call, error);
        return;
    }
    const char* verdict = NULL;
    g_variant_get(reply, "(&s)", &verdict);
    if (!g_str_equal(verdict, "yes") && !g_str_equal(verdict, "challenge")) {
        gboolean unavailable = g_str_equal(verdict, "na");
        g_autoptr(GError) refusal = g_error_new(
            G_IO_ERROR, unavailable ? G_IO_ERROR_NOT_SUPPORTED : G_IO_ERROR_PERMISSION_DENIED,
            "logind cannot %s here: %s answers \"%s\"", call->label, call->capability, verdict);
        power_finish(call, refusal);
        return;
    }
    g_dbus_connection_call(call->bus, LOGIND_NAME, LOGIND_PATH, LOGIND_MANAGER, call->action,
                           g_variant_new("(b)", TRUE), NULL, G_DBUS_CALL_FLAGS_NONE,
                           POWER_ACTION_TIMEOUT_MS, NULL, power_action_done, call);
}

static void power_bus_ready(GObject* source, GAsyncResult* async_result, gpointer user_data) {
    PowerCall* call = user_data;
    (void)source;
    g_autoptr(GError) error = NULL;
    call->bus = g_bus_get_finish(async_result, &error);
    if (!call->bus) {
        power_fail(call, error);
        return;
    }
    g_dbus_connection_call(call->bus, LOGIND_NAME, LOGIND_PATH, LOGIND_MANAGER, call->capability,
                           NULL, G_VARIANT_TYPE("(s)"), G_DBUS_CALL_FLAGS_NONE,
                           POWER_CAPABILITY_TIMEOUT_MS, NULL, power_capability_done, call);
}

/* Start the logind call for a power method. TRUE means the Operation is pending and completes
 * later; FALSE means it failed at once and `error` says why. */
gboolean gnoblin_control_power_action(GnoblinNativeControl* control, gint64 operation_id,
                                      const char* method, GVariant* arguments, GError** error) {
    for (guint i = 0; i < G_N_ELEMENTS(power_actions); i++) {
        if (!g_str_equal(power_actions[i].method, method))
            continue;
        if (g_variant_n_children(arguments) != 0) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "%s takes no arguments",
                        method);
            return FALSE;
        }
        PowerCall* call = g_new0(PowerCall, 1);
        call->control = control;
        call->operation_id = operation_id;
        call->method = power_actions[i].method;
        call->capability = power_actions[i].capability;
        call->action = power_actions[i].action;
        call->label = power_actions[i].label;
        control->pending_power_calls++;
        g_bus_get(G_BUS_TYPE_SYSTEM, NULL, power_bus_ready, call);
        return TRUE;
    }
    g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED, "%s is not a power method", method);
    return FALSE;
}
