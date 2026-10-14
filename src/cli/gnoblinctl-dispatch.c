#include "gnoblinctl-internal.h"

JsonNode* dispatch(Cli* cli, GError** error) {
    const char* command = cli->command;
    const char* action = cli->action;
    const char* op = "api";
    JsonObject* arguments = new_object();
    const char* method = NULL;
    g_autofree char* owned_method = NULL;
    g_autoptr(JsonNode) reply = NULL;

    if (is(command, "config")) {
        if (is(action, "path")) {
            json_object_unref(arguments);
            g_autofree char* path = config_path();
            return string_node(path);
        }
        if (is(action, "default")) {
            json_object_unref(arguments);
            g_autofree char* config = default_config(error);
            return config ? string_node(config) : NULL;
        }
        if (is(action, "reload"))
            method = "runtime.reload_config";
        else if (is(action, "show"))
            method = "settings";
    } else if (is(command, "window")) {
        if (is(action, "list")) {
            set_if(arguments, "app_id", option(cli, "app-id"));
            set_if(arguments, "title", option(cli, "title"));
            if (has(cli, "focused"))
                set_boolean(arguments, "focused", TRUE);
            method = "windows.list";
        } else if (is(action, "match")) {
            if (!require_count(cli, 0, 1, error))
                goto invalid;
            if (!arg(cli, 0) || is(arg(cli, 0), "active"))
                set_boolean(arguments, "focused", TRUE);
            method = "windows.list";
        } else if (is(action, "workspace")) {
            if (!require_count(cli, 1, 2, error))
                goto invalid;
            JsonObject* selector = new_object();
            if (!workspace_selector(cli, 1, selector, error)) {
                json_object_unref(selector);
                goto invalid;
            }
            if (is(arg(cli, 0), "active")) {
                set_string(arguments, "window", arg(cli, 0));
                json_object_set_object_member(arguments, "workspace", selector);
                method = "workspace.move_window";
            } else {
                set_string(arguments, "id", arg(cli, 0));
                json_object_set_object_member(arguments, "workspace", selector);
                method = "window.move_to_workspace";
            }
        } else if (is(action, "thumbnail")) {
            if (!require_count(cli, 1, 1, error))
                goto invalid;
            if (!option(cli, "output") || !*option(cli, "output")) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "window thumbnail requires --output PATH");
                goto invalid;
            }
            guint width = 320;
            guint height = 200;
            if ((option(cli, "width") && !parse_uint(option(cli, "width"), 1, 480, &width)) ||
                (option(cli, "height") && !parse_uint(option(cli, "height"), 1, 320, &height))) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "thumbnail bounds must be width 1..480 and height 1..320");
                goto invalid;
            }
            set_string(arguments, "id", arg(cli, 0));
            set_number(arguments, "width", width);
            set_number(arguments, "height", height);
            method = "window.thumbnail";
        } else if (action) {
            guint count = is(action, "move") || is(action, "resize") ? 3
                          : is(action, "monitor")                    ? 2
                                                                     : 1;
            guint minimum = count == 1 ? 0 : count;
            if (!require_count(cli, minimum, count, error))
                goto invalid;
            const char* window = arg(cli, 0) ? arg(cli, 0) : "active";
            if (is(action, "focus")) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                    "window focus requires a one-use trusted context or an XDG "
                                    "activation token; gnoblinctl cannot create either");
                goto invalid;
            }
            if (word_in("menu interactive-move interactive-resize", action)) {
                g_set_error_literal(
                    error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                    "window menu and interactive actions require a trusted shell input context; "
                    "gnoblinctl cannot create one");
                goto invalid;
            }
            if (is(action, "toggle-minimize") && is(window, "active")) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "window toggle-minimize requires a stable window ID");
                goto invalid;
            }
            gboolean active_window = is(window, "active");
            g_autofree char* resolved_window = NULL;
            if (is(window, "active")) {
                resolved_window = focused_window_id(cli, error);
                if (!resolved_window)
                    goto invalid;
                window = resolved_window;
            }
            set_string(arguments, "id", window);
            if (is(action, "close"))
                method = "window.close";
            else if (is(action, "minimize"))
                method = "window.minimize";
            else if (is(action, "unminimize"))
                method = "window.unminimize";
            else if (is(action, "restore"))
                method = "window.restore";
            else if (is(action, "restore-or-minimize"))
                method = "window.restore_or_minimize";
            else if (is(action, "toggle-minimize"))
                method = "window.toggle_minimize";
            else if (word_in("maximize unmaximize", action)) {
                method = "window.set_maximized";
                set_boolean(arguments, "enabled", is(action, "maximize"));
            } else if (word_in("fullscreen unfullscreen", action)) {
                method = "window.set_fullscreen";
                set_boolean(arguments, "enabled", is(action, "fullscreen"));
            } else if (word_in("above unabove", action)) {
                method = "window.set_above";
                set_boolean(arguments, "enabled", is(action, "above"));
            } else if (word_in("stick unstick", action)) {
                method = "window.set_sticky";
                set_boolean(arguments, "enabled", is(action, "stick"));
            } else if (is(action, "move"))
                method = "window.move";
            else if (is(action, "resize"))
                method = "window.resize";
            else if (is(action, "monitor"))
                method = "window.move_to_monitor";
            if (is(action, "move") || is(action, "resize") || is(action, "monitor")) {
                if (is(action, "monitor")) {
                    g_autofree char* indexed_monitor_id = NULL;
                    const char* monitor_id = arg(cli, 1);
                    if (active_window) {
                        guint monitor_index;
                        if (!parse_uint(monitor_id, 0, 1023, &monitor_index)) {
                            g_set_error_literal(
                                error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                "active window monitor target must be an index from "
                                "gnoblinctl monitor list");
                            goto invalid;
                        }
                        indexed_monitor_id = monitor_id_for_index(cli, monitor_index, error);
                        if (!indexed_monitor_id)
                            goto invalid;
                        monitor_id = indexed_monitor_id;
                    }
                    if (!monitor_id || !*monitor_id || strlen(monitor_id) > 128 ||
                        !g_utf8_validate(monitor_id, -1, NULL)) {
                        g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                            "monitor must be a connector ID from monitor list");
                        goto invalid;
                    }
                    set_string(arguments, "monitor", monitor_id);
                } else {
                    const char* first = is(action, "move") ? "x" : "width";
                    const char* second = is(action, "move") ? "y" : "height";
                    gint value;
                    gint low = is(action, "move") ? -100000 : 1;
                    gint high = is(action, "move") ? 100000 : 32768;
                    if (!number_arg(cli, 1, first, low, high, &value, error))
                        goto invalid;
                    set_number(arguments, first, value);
                    if (count == 3) {
                        if (!number_arg(cli, 2, second, low, high, &value, error))
                            goto invalid;
                        set_number(arguments, second, value);
                    }
                }
            }
        }
    } else if (is(command, "workspace")) {
        if (is(action, "list"))
            method = "workspaces.list";
        else if (is(action, "next") || is(action, "previous"))
            method = owned_method = g_strdup_printf("workspace.%s", action);
        else if (is(action, "create")) {
            if (!require_count(cli, 0, 0, error))
                goto invalid;
            if (!option(cli, "name")) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "workspace create requires --name");
                goto invalid;
            }
            set_string(arguments, "name", option(cli, "name"));
            set_boolean(arguments, "activate", has(cli, "activate"));
            set_if(arguments, "id", option(cli, "id"));
            method = "workspace.create";
        } else if (is(action, "rename") || is(action, "remove") || is(action, "switch") ||
                   is(action, "move-active")) {
            if (!require_count(cli, 0, is(action, "switch") ? 1 : 0, error))
                goto invalid;
            if (!workspace_selector(cli, 0, arguments, error))
                goto invalid;
            if (is(action, "rename")) {
                if (!option(cli, "name")) {
                    g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                        "workspace rename requires --name");
                    goto invalid;
                }
                set_string(arguments, "name", option(cli, "name"));
            }
            if (is(action, "move-active")) {
                JsonObject* selector = json_object_ref(arguments);
                arguments = new_object();
                json_object_set_object_member(arguments, "workspace", selector);
                set_boolean(arguments, "follow", has(cli, "follow"));
            }
            method = is(action, "move-active")
                         ? "workspace.move_active"
                         : (owned_method = g_strdup_printf("workspace.%s", action));
        }
    } else if (is(command, "animation") && action) {
        if (is(action, "get")) {
            if (!require_count(cli, 1, 1, error))
                goto invalid;
            set_string(arguments, "name", arg(cli, 0));
        } else if (is(action, "inspect") || is(action, "preview")) {
            if (!require_count(cli, 1, 1, error))
                goto invalid;
            set_string(arguments, "name", arg(cli, 0));
            set_if(arguments, "event", option(cli, "event"));
            const char* target = option(cli, "window");
            const char* type = "window";
            if (option(cli, "layer")) {
                target = option(cli, "layer");
                type = "layer";
            }
            if (option(cli, "namespace")) {
                target = option(cli, "namespace");
                type = "namespace";
            }
            set_string(arguments, "target_type", type);
            set_string(arguments, "target", target ? target : "active");
            if (is(action, "preview"))
                set_boolean(arguments, "autoplay", has(cli, "autoplay"));
        } else if (is(action, "seek") || is(action, "step")) {
            if (!require_count(cli, 2, 2, error))
                goto invalid;
            set_string(arguments, "session", arg(cli, 0));
            gint value;
            if (!number_arg(cli, 1, "value", is(action, "seek") ? 0 : 1,
                            is(action, "seek") ? 100 : 60000, &value, error))
                goto invalid;
            if (is(action, "seek"))
                json_object_set_double_member(arguments, "progress", value / 100.0);
            else
                set_number(arguments, "milliseconds", value);
        } else if (is(action, "play") || is(action, "pause") || is(action, "stop")) {
            if (!require_count(cli, 1, 1, error))
                goto invalid;
            set_string(arguments, "session", arg(cli, 0));
        }
        method = owned_method = g_strdup_printf("animation.%s", action);
    } else if (is(command, "input")) {
        if (is(action, "devices")) {
            if (!require_count(cli, 0, 0, error))
                goto invalid;
            method = "input.devices";
        } else if (is(action, "list")) {
            method = "input.sources";
        } else if (is(action, "current")) {
            method = "input.current_source";
        } else if (is(action, "select")) {
            if (!require_count(cli, 2, 2, error))
                goto invalid;
            set_string(arguments, "type", arg(cli, 0));
            set_string(arguments, "id", arg(cli, 1));
            method = "input.select";
        } else if (is(action, "orientation-lock")) {
            if (!require_count(cli, 0, 1, error))
                goto invalid;
            if (arg_count(cli) == 0) {
                method = "input.orientation_lock";
            } else {
                const char* value = arg(cli, 0);
                if (is(value, "on"))
                    set_boolean(arguments, "value", TRUE);
                else if (is(value, "off"))
                    set_boolean(arguments, "value", FALSE);
                else if (is(value, "inherit"))
                    set_string(arguments, "value", "inherit");
                else {
                    g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                        "orientation lock must be on, off, or inherit");
                    goto invalid;
                }
                method = "input.set_orientation_lock";
            }
        } else if (action) {
            method = owned_method = g_strdup_printf("input.%s", action);
        }
    } else if (is(command, "permissions")) {
        if (is(action, "check")) {
            if (!require_count(cli, 2, 2, error))
                goto invalid;
            set_string(arguments, "capability", arg(cli, 0));
            set_string(arguments, "identity", arg(cli, 1));
            method = "permissions.check";
        } else if (is(action, "policy")) {
            if (!require_count(cli, 0, 0, error))
                goto invalid;
            method = "permissions.policy";
        } else {
            method = "permissions.list";
        }
    } else if (is(command, "grant")) {
        if (is(action, "revoke")) {
            if (!require_count(cli, 2, 2, error))
                goto invalid;
            if (!is(arg(cli, 0), "screen-cast") && !is(arg(cli, 0), "remote-desktop")) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "grant kind must be screen-cast or remote-desktop");
                goto invalid;
            }
            set_string(arguments, "kind", arg(cli, 0));
            set_string(arguments, "id", arg(cli, 1));
        }
        if (action)
            method = owned_method = g_strdup_printf("grant.%s", action);
    } else if (is(command, "launch")) {
        if (is(action, "status")) {
            if (!require_count(cli, 0, 0, error))
                goto invalid;
            method = "launches.snapshot";
        } else if (is(action, "begin")) {
            if (!require_count(cli, 2, 3, error))
                goto invalid;
            set_string(arguments, "token", arg(cli, 0));
            set_string(arguments, "application", arg(cli, 1));
            gint milliseconds = 3000;
            if (arg(cli, 2) && !number_arg(cli, 2, "milliseconds", 1, 60000, &milliseconds, error))
                goto invalid;
            set_number(arguments, "milliseconds", milliseconds);
        } else if (is(action, "end")) {
            if (!require_count(cli, 1, 1, error))
                goto invalid;
            set_string(arguments, "token", arg(cli, 0));
        }
        if (action && !is(action, "status"))
            method = owned_method = g_strdup_printf("launch.%s", action);
    } else if (is(command, "layer") && is(action, "list"))
        method = "layers.list";
    else if (is(command, "monitor") && is(action, "list"))
        method = "monitors.list";
    else if (is(command, "output") && (is(action, "list") || is(action, "primary")))
        method = "outputs.list";
    else if (is(command, "focus") && is(action, "history")) {
        set_if(arguments, "workspace_id", option(cli, "workspace-id"));
        set_if(arguments, "monitor_id", option(cli, "monitor-id"));
        if (option(cli, "limit")) {
            guint limit;
            if (!parse_uint(option(cli, "limit"), 1, 256, &limit)) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "--limit must be between 1 and 256");
                goto invalid;
            }
            set_number(arguments, "limit", limit);
        }
        method = "focus.history";
    } else if (is(command, "focus") && is(action, "policy"))
        method = "focus.policy";
    else if (is(command, "capabilities"))
        method = "capabilities.list";
    else if (is(command, "privacy") && is(action, "stop-sharing"))
        method = "privacy.stop_sharing";
    else if (is(command, "privacy") && is(action, "stop-recording"))
        method = "privacy.stop_recording";
    else if (is(command, "privacy"))
        method = "privacy.state";
    else if (is(command, "reload"))
        method = "runtime.reload_config";
    else if (is(command, "session") && is(action, "activity"))
        method = "session.activity";
    else if (is(command, "session") && is(action, "lock"))
        method = "session.lock";
    else if (is(command, "status"))
        method = "session.status";
    else if (is(command, "logout") || (is(command, "session") && is(action, "logout")))
        method = "session.logout";
    else if (is(command, "ping"))
        op = "ping";
    else if (is(command, "version"))
        method = "version";
    else if (is(command, "shortcut") && is(action, "list"))
        method = "shortcuts.list";
    else if (is(command, "shortcut") && is(action, "actions")) {
        if (arg_count(cli) > 1) {
            g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                "shortcut actions accepts at most one group");
            goto invalid;
        }
        if (arg_count(cli) == 1) {
            if (!word_in("wm mutter wayland", arg(cli, 0))) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "group must be wm, mutter, or wayland");
                goto invalid;
            }
            set_string(arguments, "group", arg(cli, 0));
        }
        method = "shortcuts.actions";
    } else if (is(command, "shortcut") && is(action, "capture")) {
        g_printerr("Press a shortcut now; Escape cancels.\n");
        set_number(arguments, "timeout", cli->timeout);
        method = "shortcut.capture";
    }

    if (!method && !g_str_equal(op, "ping")) {
        g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_UNKNOWN_OPTION,
                            "unknown command or action");
        goto invalid;
    }

    reply = call_compositor(cli, op, method, arguments, error);
    if (!reply)
        return NULL;
    if (is(command, "window") && is(action, "list") && JSON_NODE_HOLDS_ARRAY(reply)) {
        JsonObject* result = json_object_new();
        json_object_set_member(result, "windows", json_node_copy(reply));
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, result);
        return node;
    }
    if (is(command, "window") && is(action, "match") && JSON_NODE_HOLDS_ARRAY(reply)) {
        g_autoptr(JsonNode) result = window_match_from_snapshot(
            json_node_get_array(reply), arg(cli, 0) ? arg(cli, 0) : "active", error);
        return result ? json_node_copy(result) : NULL;
    }
    if (is(command, "output") && is(action, "list") && JSON_NODE_HOLDS_ARRAY(reply)) {
        JsonObject* result = json_object_new();
        json_object_set_member(result, "outputs", json_node_copy(reply));
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, result);
        return node;
    }
    if (is(command, "output") && is(action, "primary") && JSON_NODE_HOLDS_ARRAY(reply)) {
        JsonArray* outputs = json_node_get_array(reply);
        for (guint i = 0; i < json_array_get_length(outputs); i++) {
            JsonObject* output = json_array_get_object_element(outputs, i);
            if (output && json_object_get_boolean_member_with_default(output, "primary", FALSE)) {
                return json_node_copy(json_array_get_element(outputs, i));
            }
        }
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "no primary output");
        return NULL;
    }
    if (is(command, "monitor") && is(action, "list") && JSON_NODE_HOLDS_ARRAY(reply)) {
        JsonObject* result = json_object_new();
        json_object_set_member(result, "monitors", json_node_copy(reply));
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, result);
        return node;
    }
    if (is(command, "workspace") && is(action, "list") && JSON_NODE_HOLDS_ARRAY(reply)) {
        JsonNode* workspaces_node = json_node_copy(reply);
        JsonArray* workspaces = json_node_get_array(workspaces_node);
        for (guint i = 0; i < json_array_get_length(workspaces); i++) {
            JsonObject* workspace = json_array_get_object_element(workspaces, i);
            JsonNode* window_count =
                workspace ? json_object_get_member(workspace, "window_count") : NULL;
            if (window_count) {
                json_object_set_member(workspace, "windows", json_node_copy(window_count));
                json_object_remove_member(workspace, "window_count");
            }
        }
        JsonObject* result = json_object_new();
        json_object_set_member(result, "workspaces", workspaces_node);
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, result);
        return node;
    }
    if (is(command, "layer") && is(action, "list") && JSON_NODE_HOLDS_ARRAY(reply)) {
        JsonObject* result = json_object_new();
        json_object_set_member(result, "layers", json_node_copy(reply));
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, result);
        return node;
    }
    if (!JSON_NODE_HOLDS_OBJECT(reply)) {
        if (is(command, "animation") && is(action, "get") && JSON_NODE_HOLDS_NULL(reply))
            return json_node_copy(reply);
        if ((is(command, "capabilities") || (is(command, "focus") && is(action, "history")) ||
             (is(command, "shortcut") && (is(action, "actions") || is(action, "list")))) &&
            JSON_NODE_HOLDS_ARRAY(reply))
            return json_node_copy(reply);
        if ((is(command, "version") || is(command, "capabilities") || is(command, "focus") ||
             (is(command, "config") && is(action, "show"))) &&
            JSON_NODE_HOLDS_NULL(reply))
            return json_node_copy(reply);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Invalid compositor response");
        return NULL;
    }
    JsonObject* response = json_node_get_object(reply);
    if (is(command, "window") && is(action, "thumbnail")) {
        const char* encoded = member_string(response, "data", NULL);
        const char* window_id = member_string(response, "window_id", NULL);
        gint64 width = json_object_get_int_member_with_default(response, "width", 0);
        gint64 height = json_object_get_int_member_with_default(response, "height", 0);
        guint requested_width = 320;
        guint requested_height = 200;
        if ((option(cli, "width") && !parse_uint(option(cli, "width"), 1, 480, &requested_width)) ||
            (option(cli, "height") &&
             !parse_uint(option(cli, "height"), 1, 320, &requested_height))) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "Invalid thumbnail dimensions");
            return NULL;
        }
        gsize image_length = 0;
        g_autofree guchar* image = encoded ? g_base64_decode(encoded, &image_length) : NULL;
        static const guchar png_signature[] = {0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n'};

        if (!window_id || !g_str_equal(window_id, arg(cli, 0)) || width < 1 || width > 480 ||
            height < 1 || height > 320 || width > requested_width || height > requested_height ||
            !image || image_length < 24 || image_length > 512 * 1024 ||
            memcmp(image, png_signature, sizeof png_signature) != 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "Compositor returned an invalid window thumbnail");
            return NULL;
        }
        guint32 png_width = ((guint32)image[16] << 24) | ((guint32)image[17] << 16) |
                            ((guint32)image[18] << 8) | image[19];
        guint32 png_height = ((guint32)image[20] << 24) | ((guint32)image[21] << 16) |
                             ((guint32)image[22] << 8) | image[23];
        if (png_width != (guint32)width || png_height != (guint32)height) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "Thumbnail dimensions do not match the PNG image");
            return NULL;
        }

        g_autoptr(GFile) output = g_file_new_for_commandline_arg(option(cli, "output"));
        if (!g_file_replace_contents(output, (const char*)image, image_length, NULL, FALSE,
                                     G_FILE_CREATE_PRIVATE, NULL, NULL, error))
            return NULL;

        JsonObject* result = json_object_new();
        json_object_set_string_member(result, "path", option(cli, "output"));
        json_object_set_int_member(result, "width", width);
        json_object_set_int_member(result, "height", height);
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, result);
        return node;
    }
    if (is(command, "version"))
        return json_node_copy(reply);
    if (is(command, "ping"))
        return string_node(member_string(response, "pong", ""));
    if (is(command, "shortcut") && is(action, "capture"))
        return string_node(member_string(response, "accelerator", ""));
    return json_node_copy(reply);

invalid:
    json_object_unref(arguments);
    return NULL;
}
