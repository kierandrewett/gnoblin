/* Native GeoClue2 agent. Authorization remains fail-closed unless Lua/native
 * control explicitly completes the request. */
#include "gnoblin-location-agent.h"

#define GEOCLUE_NAME "org.freedesktop.GeoClue2"
#define GEOCLUE_MANAGER_PATH "/org/freedesktop/GeoClue2/Manager"
#define GEOCLUE_MANAGER_IFACE "org.freedesktop.GeoClue2.Manager"
#define GEOCLUE_AGENT_PATH "/org/freedesktop/GeoClue2/Agent"
#define GEOCLUE_AGENT_IFACE "org.freedesktop.GeoClue2.Agent"
#define AUTHORIZATION_TIMEOUT_SECONDS 30

struct _GnoblinLocationAgent {
    gatomicrefcount refs;
    GMainContext* context;
    GnoblinLocationAuthorizeFunc authorize;
    GnoblinLocationStateFunc state_changed;
    gpointer user_data;
    GDestroyNotify user_data_destroy;
    gboolean started;
    gboolean stopped;
    gboolean available;
    gboolean in_use;
    gboolean manager_present;
    gboolean enabled_configured;
    gboolean enabled_value;
    gboolean accuracy_configured;
    guint accuracy_value;
    GDBusConnection* connection;
    GDBusNodeInfo* introspection;
    guint object_id;
    guint owner_watch;
    guint properties_subscription;
    GSource* retry_source;
    GHashTable* pending;
    GSettings* settings;
    gulong settings_handler;
};

struct _GnoblinLocationRequest {
    gatomicrefcount refs;
    GnoblinLocationAgent* agent;
    GDBusMethodInvocation* invocation;
    GSource* timeout;
    guint requested_accuracy;
    gboolean completed;
};

static GnoblinLocationAgent* agent_ref(GnoblinLocationAgent* agent) {
    g_atomic_ref_count_inc(&agent->refs);
    return agent;
}

static void agent_unref(GnoblinLocationAgent* agent) {
    if (!g_atomic_ref_count_dec(&agent->refs))
        return;
    if (agent->user_data_destroy)
        agent->user_data_destroy(agent->user_data);
    g_clear_pointer(&agent->pending, g_hash_table_unref);
    g_clear_pointer(&agent->settings, g_object_unref);
    g_clear_pointer(&agent->introspection, g_dbus_node_info_unref);
    g_clear_object(&agent->connection);
    g_main_context_unref(agent->context);
    g_free(agent);
}

GnoblinLocationRequest* gnoblin_location_request_ref(GnoblinLocationRequest* request) {
    g_atomic_ref_count_inc(&request->refs);
    return request;
}

void gnoblin_location_request_unref(GnoblinLocationRequest* request) {
    if (!g_atomic_ref_count_dec(&request->refs))
        return;
    g_clear_pointer(&request->timeout, g_source_unref);
    g_clear_object(&request->invocation);
    agent_unref(request->agent);
    g_free(request);
}

static guint accuracy_from_nick(const char* nick) {
    if (g_strcmp0(nick, "country") == 0)
        return 1;
    if (g_strcmp0(nick, "city") == 0)
        return 4;
    if (g_strcmp0(nick, "neighborhood") == 0)
        return 5;
    if (g_strcmp0(nick, "street") == 0)
        return 6;
    if (g_strcmp0(nick, "exact") == 0)
        return 8;
    return 0;
}

static gboolean location_enabled(GnoblinLocationAgent* agent) {
    if (agent->enabled_configured)
        return agent->enabled_value;
    return agent->settings && g_settings_get_boolean(agent->settings, "enabled");
}

static guint location_max_accuracy(GnoblinLocationAgent* agent) {
    if (!location_enabled(agent))
        return 0;
    if (agent->accuracy_configured)
        return agent->accuracy_value;
    if (!agent->settings)
        return 0;
    g_autofree char* nick = g_settings_get_string(agent->settings, "max-accuracy-level");
    return accuracy_from_nick(nick);
}

static guint clamp_accuracy(guint level, guint maximum) {
    static const guint levels[] = {0, 1, 4, 5, 6, 8};
    guint clamped = 0;
    for (guint i = 0; i < G_N_ELEMENTS(levels); i++) {
        if (levels[i] <= level && levels[i] <= maximum)
            clamped = levels[i];
    }
    return clamped;
}

static void notify_state(GnoblinLocationAgent* agent, gboolean available, gboolean in_use) {
    if (agent->available == available && agent->in_use == in_use)
        return;
    agent->available = available;
    agent->in_use = in_use;
    if (agent->state_changed)
        agent->state_changed(agent, available, in_use, agent->user_data);
}

static void complete_on_context(GnoblinLocationRequest* request, gboolean allowed,
                                guint accuracy_level) {
    GnoblinLocationAgent* agent = request->agent;
    if (request->completed)
        return;
    request->completed = TRUE;
    if (request->timeout) {
        g_source_destroy(request->timeout);
        g_clear_pointer(&request->timeout, g_source_unref);
    }
    if (agent->pending)
        g_hash_table_remove(agent->pending, request);
    guint maximum = location_max_accuracy(agent);
    if (agent->stopped || !agent->available || maximum == 0)
        allowed = FALSE;
    if (!allowed) {
        accuracy_level = 0;
    } else {
        accuracy_level = clamp_accuracy(accuracy_level, MIN(request->requested_accuracy, maximum));
        if (accuracy_level == 0)
            allowed = FALSE;
    }
    g_dbus_method_invocation_return_value(request->invocation,
                                          g_variant_new("(bu)", allowed, accuracy_level));
}

static gboolean request_timeout_cb(gpointer data) {
    GnoblinLocationRequest* request = data;
    GSource* timeout = request->timeout;
    request->timeout = NULL;
    if (timeout)
        g_source_unref(timeout);
    complete_on_context(request, FALSE, 0);
    return G_SOURCE_REMOVE;
}

typedef struct {
    GnoblinLocationRequest* request;
    gboolean allowed;
    guint accuracy_level;
} Completion;

static gboolean complete_request_cb(gpointer data) {
    Completion* completion = data;
    complete_on_context(completion->request, completion->allowed, completion->accuracy_level);
    return G_SOURCE_REMOVE;
}

static void completion_free(gpointer data) {
    Completion* completion = data;
    gnoblin_location_request_unref(completion->request);
    g_free(completion);
}

void gnoblin_location_request_complete(GnoblinLocationRequest* request, gboolean allowed,
                                       guint accuracy_level) {
    Completion* completion = g_new0(Completion, 1);
    completion->request = gnoblin_location_request_ref(request);
    completion->allowed = allowed;
    completion->accuracy_level = MIN(accuracy_level, 8);
    g_main_context_invoke_full(request->agent->context, G_PRIORITY_DEFAULT, complete_request_cb,
                               completion, completion_free);
}

static void fail_all_pending(GnoblinLocationAgent* agent) {
    GList* requests = g_hash_table_get_values(agent->pending);
    for (GList* item = requests; item; item = item->next) {
        GnoblinLocationRequest* request = gnoblin_location_request_ref(item->data);
        complete_on_context(request, FALSE, 0);
        gnoblin_location_request_unref(request);
    }
    g_list_free(requests);
}

static GVariant* get_property(GDBusConnection* connection, const char* sender,
                              const char* object_path, const char* interface_name,
                              const char* property_name, GError** error, gpointer user_data) {
    GnoblinLocationAgent* agent = user_data;
    (void)connection;
    (void)sender;
    (void)object_path;
    (void)interface_name;
    if (g_str_equal(property_name, "MaxAccuracyLevel"))
        return g_variant_new_uint32(location_max_accuracy(agent));
    g_set_error(error, G_DBUS_ERROR, G_DBUS_ERROR_UNKNOWN_PROPERTY,
                "Unknown GeoClue agent property %s", property_name);
    return NULL;
}

static void method_call(GDBusConnection* connection, const char* sender, const char* object_path,
                        const char* interface_name, const char* method_name, GVariant* parameters,
                        GDBusMethodInvocation* invocation, gpointer user_data);

static const GDBusInterfaceVTable agent_vtable = {
    .method_call = method_call,
    .get_property = get_property,
};

static gboolean request_timeout(GnoblinLocationRequest* request) {
    request->timeout = g_timeout_source_new_seconds(AUTHORIZATION_TIMEOUT_SECONDS);
    g_source_set_callback(request->timeout, request_timeout_cb,
                          gnoblin_location_request_ref(request),
                          (GDestroyNotify)gnoblin_location_request_unref);
    g_source_attach(request->timeout, request->agent->context);
    return TRUE;
}

static void method_call(GDBusConnection* connection, const char* sender, const char* object_path,
                        const char* interface_name, const char* method_name, GVariant* parameters,
                        GDBusMethodInvocation* invocation, gpointer user_data) {
    GnoblinLocationAgent* agent = user_data;
    (void)connection;
    (void)sender;
    (void)object_path;
    (void)interface_name;
    if (!g_str_equal(method_name, "AuthorizeApp")) {
        g_dbus_method_invocation_return_error(invocation, G_DBUS_ERROR, G_DBUS_ERROR_UNKNOWN_METHOD,
                                              "Unknown GeoClue agent method %s", method_name);
        return;
    }

    const char* app_id;
    guint requested_accuracy;
    g_variant_get(parameters, "(&su)", &app_id, &requested_accuracy);
    GnoblinLocationRequest* request = g_new0(GnoblinLocationRequest, 1);
    g_atomic_ref_count_init(&request->refs);
    request->agent = agent_ref(agent);
    request->invocation = g_object_ref(invocation);
    request->requested_accuracy = requested_accuracy;
    if (agent->stopped || !agent->available || !agent->authorize ||
        location_max_accuracy(agent) == 0) {
        complete_on_context(request, FALSE, 0);
        gnoblin_location_request_unref(request);
        return;
    }
    g_hash_table_add(agent->pending, gnoblin_location_request_ref(request));
    request_timeout(request);
    agent->authorize(agent, app_id, requested_accuracy, request, agent->user_data);
    gnoblin_location_request_unref(request);
}

static const char agent_xml[] =
    "<node><interface name='org.freedesktop.GeoClue2.Agent'>"
    "<method name='AuthorizeApp'><arg name='desktop_id' type='s' direction='in'/>"
    "<arg name='req_accuracy_level' type='u' direction='in'/>"
    "<arg name='allowed' type='b' direction='out'/>"
    "<arg name='accuracy_level' type='u' direction='out'/></method>"
    "<property name='MaxAccuracyLevel' type='u' access='read'/>"
    "</interface></node>";

static void get_in_use(GnoblinLocationAgent* agent);

static void add_agent_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    GnoblinLocationAgent* agent = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source), result, &error);
    if (!agent->stopped && agent->manager_present && reply) {
        notify_state(agent, TRUE, FALSE);
        get_in_use(agent);
    } else if (!agent->stopped && error) {
        g_debug("GeoClue AddAgent failed: %s", error->message);
        notify_state(agent, FALSE, FALSE);
    }
    agent_unref(agent);
}

static void manager_name_appeared(GDBusConnection* connection, const char* name, const char* owner,
                                  gpointer user_data) {
    GnoblinLocationAgent* agent = user_data;
    (void)name;
    (void)owner;
    if (agent->stopped)
        return;
    agent->manager_present = TRUE;
    g_dbus_connection_call(connection, GEOCLUE_NAME, GEOCLUE_MANAGER_PATH, GEOCLUE_MANAGER_IFACE,
                           "AddAgent", g_variant_new("(s)", "gnoblin"), NULL,
                           G_DBUS_CALL_FLAGS_NONE, 5000, NULL, add_agent_done, agent_ref(agent));
}

static void manager_name_vanished(GDBusConnection* connection, const char* name,
                                  gpointer user_data) {
    GnoblinLocationAgent* agent = user_data;
    (void)connection;
    (void)name;
    if (agent->stopped)
        return;
    agent->manager_present = FALSE;
    notify_state(agent, FALSE, FALSE);
    fail_all_pending(agent);
}

static void in_use_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    GnoblinLocationAgent* agent = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source), result, &error);
    if (!agent->stopped && reply && agent->available) {
        GVariant* value;
        g_variant_get(reply, "(v)", &value);
        notify_state(agent, TRUE, g_variant_get_boolean(value));
        g_variant_unref(value);
    }
    agent_unref(agent);
}

static void get_in_use(GnoblinLocationAgent* agent) {
    g_dbus_connection_call(
        agent->connection, GEOCLUE_NAME, GEOCLUE_MANAGER_PATH, "org.freedesktop.DBus.Properties",
        "Get", g_variant_new("(ss)", GEOCLUE_MANAGER_IFACE, "InUse"), G_VARIANT_TYPE("(v)"),
        G_DBUS_CALL_FLAGS_NONE, 5000, NULL, in_use_done, agent_ref(agent));
}

static void properties_changed(GDBusConnection* connection, const char* sender_name,
                               const char* object_path, const char* interface_name,
                               const char* signal_name, GVariant* parameters, gpointer user_data) {
    GnoblinLocationAgent* agent = user_data;
    (void)connection;
    (void)sender_name;
    (void)interface_name;
    (void)signal_name;
    if (agent->stopped || !agent->available || !g_str_equal(object_path, GEOCLUE_MANAGER_PATH))
        return;
    GVariant* changed;
    const char* changed_iface;
    GVariant* invalidated;
    g_variant_get(parameters, "(&s@a{sv}@as)", &changed_iface, &changed, &invalidated);
    g_variant_unref(invalidated);
    if (!g_str_equal(changed_iface, GEOCLUE_MANAGER_IFACE)) {
        g_variant_unref(changed);
        return;
    }
    GVariant* value = g_variant_lookup_value(changed, "InUse", G_VARIANT_TYPE_BOOLEAN);
    if (value) {
        notify_state(agent, TRUE, g_variant_get_boolean(value));
        g_variant_unref(value);
    }
    g_variant_unref(changed);
}

static gboolean retry_connect(gpointer data);
static void connect_done(GObject* source, GAsyncResult* result, gpointer user_data);

static void get_system_bus(GnoblinLocationAgent* agent) {
    g_main_context_push_thread_default(agent->context);
    g_bus_get(G_BUS_TYPE_SYSTEM, NULL, connect_done, agent_ref(agent));
    g_main_context_pop_thread_default(agent->context);
}

static void connect_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    GnoblinLocationAgent* agent = user_data;
    g_autoptr(GError) error = NULL;
    GDBusConnection* connection = g_bus_get_finish(result, &error);
    (void)source;
    if (agent->stopped) {
        g_clear_object(&connection);
        agent_unref(agent);
        return;
    }
    if (!connection) {
        g_debug("Could not connect GeoClue agent to system bus: %s", error->message);
        if (!agent->retry_source) {
            agent->retry_source = g_timeout_source_new_seconds(5);
            g_source_set_priority(agent->retry_source, G_PRIORITY_DEFAULT);
            g_source_set_callback(agent->retry_source, retry_connect, agent_ref(agent),
                                  (GDestroyNotify)agent_unref);
            g_source_attach(agent->retry_source, agent->context);
        }
        agent_unref(agent);
        return;
    }
    if (agent->retry_source) {
        g_source_destroy(agent->retry_source);
        g_clear_pointer(&agent->retry_source, g_source_unref);
    }
    agent->connection = connection;
    g_autoptr(GError) parse_error = NULL;
    agent->introspection = g_dbus_node_info_new_for_xml(agent_xml, &parse_error);
    if (!agent->introspection) {
        g_warning("Could not parse GeoClue agent interface: %s", parse_error->message);
        agent_unref(agent);
        return;
    }
    agent->object_id = g_dbus_connection_register_object(
        connection, GEOCLUE_AGENT_PATH, agent->introspection->interfaces[0], &agent_vtable,
        agent_ref(agent), (GDestroyNotify)agent_unref, &parse_error);
    if (!agent->object_id) {
        g_warning("Could not export GeoClue agent: %s", parse_error->message);
        agent_unref(agent);
        return;
    }
    agent->owner_watch = g_bus_watch_name_on_connection(
        connection, GEOCLUE_NAME, G_BUS_NAME_WATCHER_FLAGS_NONE, manager_name_appeared,
        manager_name_vanished, agent_ref(agent), (GDestroyNotify)agent_unref);
    agent->properties_subscription = g_dbus_connection_signal_subscribe(
        connection, GEOCLUE_NAME, "org.freedesktop.DBus.Properties", "PropertiesChanged",
        GEOCLUE_MANAGER_PATH, GEOCLUE_MANAGER_IFACE, G_DBUS_SIGNAL_FLAGS_NONE, properties_changed,
        agent_ref(agent), (GDestroyNotify)agent_unref);
    agent_unref(agent);
}

static gboolean retry_connect(gpointer data) {
    GnoblinLocationAgent* agent = data;
    if (agent->stopped) {
        return G_SOURCE_REMOVE;
    }
    get_system_bus(agent);
    return G_SOURCE_CONTINUE;
}

static gboolean begin_start(gpointer data) {
    GnoblinLocationAgent* agent = data;
    if (!agent->stopped && !agent->started) {
        agent->started = TRUE;
        get_system_bus(agent);
    }
    return G_SOURCE_REMOVE;
}

static gboolean begin_stop(gpointer data) {
    GnoblinLocationAgent* agent = data;
    if (!agent->stopped) {
        agent->stopped = TRUE;
        if (agent->retry_source) {
            g_source_destroy(agent->retry_source);
            g_clear_pointer(&agent->retry_source, g_source_unref);
        }
        if (agent->owner_watch) {
            g_bus_unwatch_name(agent->owner_watch);
            agent->owner_watch = 0;
        }
        if (agent->properties_subscription) {
            g_dbus_connection_signal_unsubscribe(agent->connection, agent->properties_subscription);
            agent->properties_subscription = 0;
        }
        if (agent->object_id) {
            g_dbus_connection_unregister_object(agent->connection, agent->object_id);
            agent->object_id = 0;
        }
        fail_all_pending(agent);
    }
    return G_SOURCE_REMOVE;
}

static void emit_max_accuracy_changed(GnoblinLocationAgent* agent) {
    if (!agent->connection || !agent->object_id)
        return;
    GVariantBuilder changed;
    g_variant_builder_init(&changed, G_VARIANT_TYPE("a{sv}"));
    g_variant_builder_add(&changed, "{sv}", "MaxAccuracyLevel",
                          g_variant_new_uint32(location_max_accuracy(agent)));
    g_dbus_connection_emit_signal(agent->connection, NULL, GEOCLUE_AGENT_PATH,
                                  "org.freedesktop.DBus.Properties", "PropertiesChanged",
                                  g_variant_new("(s@a{sv}@as)", GEOCLUE_AGENT_IFACE,
                                                g_variant_builder_end(&changed),
                                                g_variant_new_strv(NULL, 0)),
                                  NULL);
}

static void settings_changed(GSettings* settings, const char* key, gpointer user_data) {
    GnoblinLocationAgent* agent = user_data;
    (void)settings;
    (void)key;
    emit_max_accuracy_changed(agent);
}

typedef struct {
    GnoblinLocationAgent* agent;
    gboolean enabled_set;
    gboolean enabled;
    gboolean accuracy_set;
    GnoblinLocationAccuracy accuracy;
} LocationPolicyUpdate;

static gboolean apply_policy(gpointer data) {
    LocationPolicyUpdate* update = data;
    GnoblinLocationAgent* agent = update->agent;
    guint old_maximum = location_max_accuracy(agent);
    agent->enabled_configured = update->enabled_set;
    agent->enabled_value = update->enabled;
    agent->accuracy_configured =
        update->accuracy_set && update->accuracy != GNOBLIN_LOCATION_ACCURACY_INHERIT;
    agent->accuracy_value = update->accuracy;
    if (old_maximum != location_max_accuracy(agent))
        emit_max_accuracy_changed(agent);
    return G_SOURCE_REMOVE;
}

static void policy_update_free(gpointer data) {
    LocationPolicyUpdate* update = data;
    agent_unref(update->agent);
    g_free(update);
}

GnoblinLocationAgent* gnoblin_location_agent_new(GMainContext* context,
                                                 GnoblinLocationAuthorizeFunc authorize,
                                                 GnoblinLocationStateFunc state_changed,
                                                 gpointer user_data,
                                                 GDestroyNotify user_data_destroy) {
    GnoblinLocationAgent* agent = g_new0(GnoblinLocationAgent, 1);
    g_atomic_ref_count_init(&agent->refs);
    agent->context = g_main_context_ref(context ? context : g_main_context_default());
    agent->authorize = authorize;
    agent->state_changed = state_changed;
    agent->user_data = user_data;
    agent->user_data_destroy = user_data_destroy;
    agent->pending = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL,
                                           (GDestroyNotify)gnoblin_location_request_unref);
    GSettingsSchemaSource* source = g_settings_schema_source_get_default();
    GSettingsSchema* schema =
        source ? g_settings_schema_source_lookup(source, "org.gnome.system.location", TRUE) : NULL;
    if (schema) {
        agent->settings = g_settings_new_full(schema, NULL, NULL);
        g_settings_schema_unref(schema);
    } else {
        g_debug("GSettings schema org.gnome.system.location is unavailable; Lua policy can still "
                "supply location defaults");
    }
    if (agent->settings)
        agent->settings_handler =
            g_signal_connect(agent->settings, "changed", G_CALLBACK(settings_changed), agent);
    return agent;
}

void gnoblin_location_agent_start(GnoblinLocationAgent* agent) {
    g_main_context_invoke_full(agent->context, G_PRIORITY_DEFAULT, begin_start, agent_ref(agent),
                               (GDestroyNotify)agent_unref);
}

void gnoblin_location_agent_set_policy(GnoblinLocationAgent* agent, gboolean enabled_set,
                                       gboolean enabled, gboolean accuracy_set,
                                       GnoblinLocationAccuracy accuracy) {
    g_return_if_fail(agent != NULL);
    g_return_if_fail(!accuracy_set || accuracy == GNOBLIN_LOCATION_ACCURACY_INHERIT ||
                     accuracy == GNOBLIN_LOCATION_ACCURACY_COUNTRY ||
                     accuracy == GNOBLIN_LOCATION_ACCURACY_CITY ||
                     accuracy == GNOBLIN_LOCATION_ACCURACY_NEIGHBORHOOD ||
                     accuracy == GNOBLIN_LOCATION_ACCURACY_STREET ||
                     accuracy == GNOBLIN_LOCATION_ACCURACY_EXACT);
    LocationPolicyUpdate* update = g_new0(LocationPolicyUpdate, 1);
    update->agent = agent_ref(agent);
    update->enabled_set = enabled_set;
    update->enabled = enabled;
    update->accuracy_set = accuracy_set;
    update->accuracy = accuracy;
    g_main_context_invoke_full(agent->context, G_PRIORITY_DEFAULT, apply_policy, update,
                               policy_update_free);
}

void gnoblin_location_agent_stop(GnoblinLocationAgent* agent) {
    g_main_context_invoke_full(agent->context, G_PRIORITY_HIGH, begin_stop, agent_ref(agent),
                               (GDestroyNotify)agent_unref);
}

void gnoblin_location_agent_free(GnoblinLocationAgent* agent) {
    if (!agent)
        return;
    gnoblin_location_agent_stop(agent);
    agent_unref(agent);
}
