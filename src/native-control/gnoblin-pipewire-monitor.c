/* PipeWire microphone activity monitor for the native Gnoblin runtime. */
#include "gnoblin-pipewire-monitor.h"

#include <pipewire/core.h>
#include <pipewire/context.h>
#include <pipewire/node.h>
#include <pipewire/pipewire.h>
#include <pipewire/proxy.h>
#include <pipewire/thread-loop.h>
#include <pipewire/keys.h>
#include <spa/utils/dict.h>
#include <spa/utils/hook.h>

typedef struct {
    GnoblinPipewireMonitor* monitor;
    struct pw_node* node;
    struct spa_hook listener;
    gboolean running;
} NodeBinding;

struct _GnoblinPipewireMonitor {
    gatomicrefcount refs;
    GMainContext* context;
    GnoblinPipewireMonitorCallback callback;
    gpointer user_data;
    GDestroyNotify user_data_destroy;
    GMutex callback_mutex;
    GCond callback_cond;
    guint callbacks_running;

    gboolean started;
    gboolean stopping;
    GSource* retry_source;

    struct pw_thread_loop* loop;
    struct pw_context* pw_context;
    struct pw_core* core;
    struct pw_registry* registry;
    struct spa_hook core_listener;
    struct spa_hook registry_listener;
    GHashTable* nodes;
    gboolean available;
    gboolean microphone_in_use;
    gint connection_lost;
    gboolean loop_started;
    int initial_sync_seq;
};

typedef struct {
    GnoblinPipewireMonitor* monitor;
    gboolean available;
    gboolean microphone_in_use;
} Notification;

static GnoblinPipewireMonitor* monitor_ref(GnoblinPipewireMonitor* monitor) {
    g_atomic_ref_count_inc(&monitor->refs);
    return monitor;
}

static void monitor_unref(GnoblinPipewireMonitor* monitor) {
    if (!g_atomic_ref_count_dec(&monitor->refs))
        return;

    if (monitor->user_data_destroy)
        monitor->user_data_destroy(monitor->user_data);
    g_clear_pointer(&monitor->nodes, g_hash_table_unref);
    g_cond_clear(&monitor->callback_cond);
    g_mutex_clear(&monitor->callback_mutex);
    g_main_context_unref(monitor->context);
    g_free(monitor);
}

static gboolean notify_on_context(gpointer data) {
    Notification* notification = data;
    GnoblinPipewireMonitor* monitor = notification->monitor;

    g_mutex_lock(&monitor->callback_mutex);
    gboolean deliver = monitor->started && monitor->callback;
    if (deliver)
        monitor->callbacks_running++;
    g_mutex_unlock(&monitor->callback_mutex);

    if (deliver) {
        monitor->callback(monitor, notification->available, notification->microphone_in_use,
                          monitor->user_data);
        g_mutex_lock(&monitor->callback_mutex);
        monitor->callbacks_running--;
        g_cond_broadcast(&monitor->callback_cond);
        g_mutex_unlock(&monitor->callback_mutex);
    }
    return G_SOURCE_REMOVE;
}

static void notification_free(gpointer data) {
    Notification* notification = data;
    monitor_unref(notification->monitor);
    g_free(notification);
}

static void attach_notification(GnoblinPipewireMonitor* monitor, Notification* notification) {
    GSource* source = g_idle_source_new();
    g_source_set_priority(source, G_PRIORITY_DEFAULT);
    g_source_set_callback(source, notify_on_context, notification, notification_free);
    g_source_attach(source, monitor->context);
    g_source_unref(source);
}

static void queue_notification(GnoblinPipewireMonitor* monitor, gboolean available,
                               gboolean microphone_in_use) {
    Notification* notification;

    if (monitor->available == available && monitor->microphone_in_use == microphone_in_use)
        return;

    monitor->available = available;
    monitor->microphone_in_use = microphone_in_use;
    notification = g_new0(Notification, 1);
    notification->monitor = monitor_ref(monitor);
    notification->available = available;
    notification->microphone_in_use = microphone_in_use;
    attach_notification(monitor, notification);
}

static void queue_current_state(GnoblinPipewireMonitor* monitor) {
    Notification* notification = g_new0(Notification, 1);
    notification->monitor = monitor_ref(monitor);
    notification->available = monitor->available;
    notification->microphone_in_use = monitor->microphone_in_use;
    attach_notification(monitor, notification);
}

static gboolean any_node_running(GnoblinPipewireMonitor* monitor) {
    GHashTableIter iter;
    gpointer value;

    g_hash_table_iter_init(&iter, monitor->nodes);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        if (((NodeBinding*)value)->running)
            return TRUE;
    }
    return FALSE;
}

static void update_activity(GnoblinPipewireMonitor* monitor) {
    queue_notification(monitor, monitor->available,
                       monitor->available && any_node_running(monitor));
}

static void on_node_info(void* data, const struct pw_node_info* info) {
    NodeBinding* binding = data;
    GnoblinPipewireMonitor* monitor;

    if (!binding || !info)
        return;
    monitor = binding->monitor;
    binding->running = info->state == PW_NODE_STATE_RUNNING;
    update_activity(monitor);
}

static void node_binding_free(gpointer data) {
    NodeBinding* binding = data;
    if (!binding)
        return;
    spa_hook_remove(&binding->listener);
    if (binding->node)
        pw_proxy_destroy((struct pw_proxy*)binding->node);
    g_free(binding);
}

static const struct pw_node_events node_events = {
    PW_VERSION_NODE_EVENTS,
    .info = on_node_info,
};

static void remove_node(GnoblinPipewireMonitor* monitor, uint32_t id) {
    NodeBinding* binding = g_hash_table_lookup(monitor->nodes, GUINT_TO_POINTER(id));
    if (!binding)
        return;

    g_hash_table_remove(monitor->nodes, GUINT_TO_POINTER(id));
    update_activity(monitor);
}

static void remove_node_for_proxy(GnoblinPipewireMonitor* monitor, uint32_t proxy_id) {
    GHashTableIter iter;
    gpointer value;

    g_hash_table_iter_init(&iter, monitor->nodes);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NodeBinding* binding = value;
        if (pw_proxy_get_id((struct pw_proxy*)binding->node) == proxy_id) {
            g_hash_table_iter_remove(&iter);
            update_activity(monitor);
            return;
        }
    }
}

static void on_global(void* data, uint32_t id, uint32_t permissions, const char* type,
                      uint32_t version, const struct spa_dict* props) {
    GnoblinPipewireMonitor* monitor = data;
    const char* media_class;
    NodeBinding* binding;
    (void)permissions;

    if (!g_str_equal(type, PW_TYPE_INTERFACE_Node) || !props)
        return;
    media_class = spa_dict_lookup(props, PW_KEY_MEDIA_CLASS);
    if (!media_class || !g_str_equal(media_class, "Stream/Input/Audio"))
        return;

    binding = g_new0(NodeBinding, 1);
    binding->monitor = monitor;
    binding->node = pw_registry_bind(monitor->registry, id, PW_TYPE_INTERFACE_Node,
                                     MIN(version, PW_VERSION_NODE), 0);
    if (!binding->node) {
        g_free(binding);
        return;
    }
    g_hash_table_insert(monitor->nodes, GUINT_TO_POINTER(id), binding);
    if (pw_node_add_listener(binding->node, &binding->listener, &node_events, binding) < 0) {
        g_warning("gnoblin-pipewire-monitor: cannot observe capture node %u", id);
        g_atomic_int_set(&monitor->connection_lost, TRUE);
        queue_notification(monitor, FALSE, FALSE);
        remove_node(monitor, id);
    }
}

static void on_global_remove(void* data, uint32_t id) {
    remove_node(data, id);
}

static const struct pw_registry_events registry_events = {
    PW_VERSION_REGISTRY_EVENTS,
    .global = on_global,
    .global_remove = on_global_remove,
};

static void on_core_info(void* data, const struct pw_core_info* info) {
    GnoblinPipewireMonitor* monitor = data;
    (void)info;
    g_atomic_int_set(&monitor->connection_lost, FALSE);
}

static void on_core_done(void* data, uint32_t id, int seq) {
    GnoblinPipewireMonitor* monitor = data;
    if (!g_atomic_int_get(&monitor->connection_lost) && id == PW_ID_CORE &&
        seq == monitor->initial_sync_seq)
        queue_notification(monitor, TRUE, any_node_running(monitor));
}

static void on_core_error(void* data, uint32_t id, int seq, int res, const char* message) {
    GnoblinPipewireMonitor* monitor = data;
    (void)seq;
    (void)res;
    (void)message;

    if (id != PW_ID_CORE &&
        (!monitor->registry || id != pw_proxy_get_id((struct pw_proxy*)monitor->registry))) {
        remove_node_for_proxy(monitor, id);
        return;
    }

    g_atomic_int_set(&monitor->connection_lost, TRUE);
    queue_notification(monitor, FALSE, FALSE);
    if (monitor->nodes)
        g_hash_table_remove_all(monitor->nodes);
}

static const struct pw_core_events core_events = {
    PW_VERSION_CORE_EVENTS,
    .info = on_core_info,
    .done = on_core_done,
    .error = on_core_error,
};

static void disconnect_pipewire(GnoblinPipewireMonitor* monitor) {
    if (!monitor->loop)
        return;

    if (monitor->loop_started) {
        pw_thread_loop_stop(monitor->loop);
        monitor->loop_started = FALSE;
    }
    pw_thread_loop_lock(monitor->loop);

    if (monitor->nodes)
        g_hash_table_remove_all(monitor->nodes);
    if (monitor->registry) {
        spa_hook_remove(&monitor->registry_listener);
        pw_proxy_destroy((struct pw_proxy*)monitor->registry);
        monitor->registry = NULL;
    }
    if (monitor->core) {
        spa_hook_remove(&monitor->core_listener);
        pw_core_disconnect(monitor->core);
        monitor->core = NULL;
    }
    if (monitor->pw_context) {
        pw_context_destroy(monitor->pw_context);
        monitor->pw_context = NULL;
    }
    pw_thread_loop_unlock(monitor->loop);
    pw_thread_loop_destroy(monitor->loop);
    monitor->loop = NULL;
}

static gboolean connect_pipewire(GnoblinPipewireMonitor* monitor) {
    g_atomic_int_set(&monitor->connection_lost, FALSE);
    monitor->nodes = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, node_binding_free);
    monitor->loop = pw_thread_loop_new("gnoblin-microphone-monitor", NULL);
    if (!monitor->loop)
        goto fail;
    monitor->pw_context = pw_context_new(pw_thread_loop_get_loop(monitor->loop), NULL, 0);
    if (!monitor->pw_context)
        goto fail;
    if (pw_thread_loop_start(monitor->loop) < 0)
        goto fail;
    monitor->loop_started = TRUE;

    pw_thread_loop_lock(monitor->loop);
    monitor->core = pw_context_connect(monitor->pw_context, NULL, 0);
    if (monitor->core) {
        pw_core_add_listener(monitor->core, &monitor->core_listener, &core_events, monitor);
        monitor->registry = pw_core_get_registry(monitor->core, PW_VERSION_REGISTRY, 0);
    }
    if (monitor->registry)
        pw_registry_add_listener(monitor->registry, &monitor->registry_listener, &registry_events,
                                 monitor);
    if (monitor->registry)
        monitor->initial_sync_seq = pw_core_sync(monitor->core, PW_ID_CORE, 0);
    if (monitor->registry && monitor->initial_sync_seq < 0)
        g_atomic_int_set(&monitor->connection_lost, TRUE);
    pw_thread_loop_unlock(monitor->loop);

    if (!monitor->core || !monitor->registry)
        goto fail;
    return TRUE;

fail:
    disconnect_pipewire(monitor);
    if (monitor->nodes) {
        g_hash_table_unref(monitor->nodes);
        monitor->nodes = NULL;
    }
    g_atomic_int_set(&monitor->connection_lost, FALSE);
    return FALSE;
}

static gboolean retry_connection(gpointer data) {
    GnoblinPipewireMonitor* monitor = data;

    g_mutex_lock(&monitor->callback_mutex);
    if (!monitor->started) {
        g_mutex_unlock(&monitor->callback_mutex);
        return G_SOURCE_REMOVE;
    }
    if (!monitor->loop || g_atomic_int_get(&monitor->connection_lost)) {
        disconnect_pipewire(monitor);
        if (monitor->nodes) {
            g_hash_table_unref(monitor->nodes);
            monitor->nodes = NULL;
        }
        queue_notification(monitor, FALSE, FALSE);
        connect_pipewire(monitor);
    }
    g_mutex_unlock(&monitor->callback_mutex);
    return G_SOURCE_CONTINUE;
}

GnoblinPipewireMonitor* gnoblin_pipewire_monitor_new(GMainContext* context,
                                                     GnoblinPipewireMonitorCallback callback,
                                                     gpointer user_data,
                                                     GDestroyNotify user_data_destroy) {
    GnoblinPipewireMonitor* monitor = g_new0(GnoblinPipewireMonitor, 1);
    g_atomic_ref_count_init(&monitor->refs);
    g_mutex_init(&monitor->callback_mutex);
    g_cond_init(&monitor->callback_cond);
    monitor->context = g_main_context_ref(context ? context : g_main_context_default());
    monitor->callback = callback;
    monitor->user_data = user_data;
    monitor->user_data_destroy = user_data_destroy;
    return monitor;
}

gboolean gnoblin_pipewire_monitor_start(GnoblinPipewireMonitor* monitor) {
    g_return_val_if_fail(monitor != NULL, FALSE);
    g_mutex_lock(&monitor->callback_mutex);
    if (monitor->started || monitor->stopping) {
        g_mutex_unlock(&monitor->callback_mutex);
        return FALSE;
    }

    static gsize pipewire_initialized;
    if (g_once_init_enter(&pipewire_initialized)) {
        pw_init(NULL, NULL);
        g_once_init_leave(&pipewire_initialized, 1);
    }

    monitor->started = TRUE;
    queue_current_state(monitor);
    connect_pipewire(monitor);

    monitor->retry_source = g_timeout_source_new_seconds(2);
    g_source_set_callback(monitor->retry_source, retry_connection, monitor_ref(monitor),
                          (GDestroyNotify)monitor_unref);
    g_source_attach(monitor->retry_source, monitor->context);
    g_mutex_unlock(&monitor->callback_mutex);
    return TRUE;
}

void gnoblin_pipewire_monitor_stop(GnoblinPipewireMonitor* monitor) {
    g_return_if_fail(monitor != NULL);
    g_mutex_lock(&monitor->callback_mutex);
    if (!monitor->started) {
        g_mutex_unlock(&monitor->callback_mutex);
        return;
    }

    monitor->started = FALSE;
    monitor->stopping = TRUE;
    gboolean wait_for_callbacks = !g_main_context_is_owner(monitor->context);
    while (wait_for_callbacks && monitor->callbacks_running > 0)
        g_cond_wait(&monitor->callback_cond, &monitor->callback_mutex);
    if (monitor->retry_source) {
        g_source_destroy(monitor->retry_source);
        g_source_unref(monitor->retry_source);
        monitor->retry_source = NULL;
    }
    disconnect_pipewire(monitor);
    if (monitor->nodes) {
        g_hash_table_unref(monitor->nodes);
        monitor->nodes = NULL;
    }
    g_atomic_int_set(&monitor->connection_lost, FALSE);
    monitor->available = FALSE;
    monitor->microphone_in_use = FALSE;
    monitor->stopping = FALSE;
    g_mutex_unlock(&monitor->callback_mutex);
}

void gnoblin_pipewire_monitor_free(GnoblinPipewireMonitor* monitor) {
    if (!monitor)
        return;
    gnoblin_pipewire_monitor_stop(monitor);
    monitor_unref(monitor);
}
