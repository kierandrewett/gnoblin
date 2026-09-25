import GObject from "gi://GObject";
import * as Workspaces from "./gnoblinWorkspaces.js";

function canonicalSignalName(signal) {
    return signal.replaceAll("_", "-");
}

function objectType(object) {
    try {
        return object?.constructor?.$gtype?.name ?? GObject.type_name_from_instance(object);
    } catch {
        return typeof object;
    }
}

function windowDetails(window, prefix, payload) {
    try {
        payload[`${prefix}app_id`] = window.get_gtk_application_id() || "";
        payload[`${prefix}wm_class`] = window.get_wm_class() || "";
        payload[`${prefix}title`] = window.get_title() || "";
        if (typeof window.get_stable_sequence === "function")
            payload[`${prefix}stable_sequence`] = window.get_stable_sequence();
    } catch {
        // Signal arguments can outlive a window while Mutter is tearing it down.
    }
}

function workspaceDetails(workspace, prefix, payload, snapshot = null) {
    try {
        const details = Workspaces.describe(workspace);
        payload[`${prefix}id`] = details.id;
        payload[`${prefix}number`] = details.number;
        payload[`${prefix}name`] = details.name;
        payload[`${prefix}active`] = details.active;
        payload[`${prefix}window_count`] = details.windows;
    } catch {
        if (!snapshot) return;
        payload[`${prefix}id`] = snapshot.id;
        payload[`${prefix}number`] = snapshot.number;
        payload[`${prefix}name`] = snapshot.name;
        payload[`${prefix}active`] = snapshot.active;
        payload[`${prefix}window_count`] = snapshot.windows;
    }
}

function argumentValue(value, index, typeName, payload) {
    const key = `arg${index}`;
    if (typeName) payload[`${key}_type`] = typeName;
    if (value === null || value === undefined) return;

    switch (typeof value) {
        case "boolean":
        case "string":
            payload[key] = value;
            return;
        case "number":
            if (Number.isFinite(value)) payload[key] = value;
            return;
        case "bigint":
            payload[key] = value.toString();
            return;
        default:
            break;
    }

    if (Array.isArray(value)) {
        const values = value.filter((item) => ["boolean", "number", "string"].includes(typeof item));
        payload[key] = JSON.stringify(values);
        return;
    }

    if (typeof value.name === "string") payload[`${key}_name`] = value.name;
    if (typeof value.get_gtk_application_id === "function") windowDetails(value, `${key}_`, payload);
    else if (typeof value.list_windows === "function" && typeof value.activate === "function")
        workspaceDetails(value, `${key}_workspace_`, payload);
    else payload[`${key}_type`] ??= objectType(value);
}

function describeEvent(emitter, source, signal, args, parameterTypes, workspace = null, snapshot = null) {
    const payload = { source, signal };
    if (source === "mutter.window") windowDetails(emitter, "window_", payload);
    if (source === "mutter.workspace") workspaceDetails(emitter, "workspace_", payload, snapshot);
    else if (workspace) workspaceDetails(workspace, "workspace_", payload, snapshot);
    args.forEach((value, index) => argumentValue(value, index, parameterTypes[index]?.name, payload));
    return payload;
}

export class MutterEventForwarder {
    constructor(config) {
        this._config = config;
        this._objects = new Map();
        this._connections = [];
        this._hooks = new Map();
        this._windows = new Set();
        this._workspaces = new Set();
        this._workspaceOrder = [];
        this._workspaceSnapshots = new Map();

        const display = global.display;
        const workspaceManager = global.workspace_manager;
        const backend = global.backend;
        const monitorManager = backend.get_monitor_manager?.();
        const cursorTracker = backend.get_cursor_tracker?.();
        this._watchObject("mutter.display", display);
        this._watchObject("mutter.workspace-manager", workspaceManager);
        this._watchObject("mutter.backend", backend);
        this._watchObject("mutter.monitor-manager", monitorManager);
        this._watchObject("mutter.cursor-tracker", cursorTracker);
        this._refreshWorkspaceSnapshots();
        for (const workspace of this._workspaceOrder) this._watchWorkspace(workspace);

        this._connect(display, "window-created", (_display, window) => this._watchWindow(window));
        this._connect(workspaceManager, "workspace-added", (_manager, number) => {
            this._watchWorkspace(workspaceManager.get_workspace_by_index(number));
            this._refreshWorkspaceSnapshots();
        });
        this._connect(workspaceManager, "workspace-removed", (_manager, number) => {
            const removed = this._workspaceOrder[number];
            if (removed) {
                this._unwatchObject(removed, this._workspaces);
                this._workspaceSnapshots.delete(removed);
            }
            this._refreshWorkspaceSnapshots();
        });

        for (const window of display.list_all_windows()) this._watchWindow(window);
    }

    _connect(object, signal, callback) {
        if (!object) return;
        try {
            this._connections.push([object, object.connect(signal, callback)]);
        } catch (error) {
            console.warn(`gnoblin events: could not watch ${signal}: ${error.message}`);
        }
    }

    _watchWindow(window) {
        if (!window || this._windows.has(window)) return;
        this._windows.add(window);
        this._watchObject("mutter.window", window);
        this._connect(window, "unmanaged", () => this._unwatchObject(window, this._windows));
    }

    _watchWorkspace(workspace) {
        if (!workspace || this._workspaces.has(workspace)) return;
        this._workspaces.add(workspace);
        this._snapshotWorkspace(workspace);
        this._watchObject("mutter.workspace", workspace);
    }

    _snapshotWorkspace(workspace) {
        try {
            this._workspaceSnapshots.set(workspace, Workspaces.describe(workspace));
        } catch {
            // A workspace that has already been removed has no current record.
        }
    }

    _refreshWorkspaceSnapshots() {
        const manager = global.workspace_manager;
        this._workspaceOrder = Array.from({ length: manager.n_workspaces }, (_, index) =>
            manager.get_workspace_by_index(index),
        );
        for (const workspace of this._workspaceOrder) this._snapshotWorkspace(workspace);
    }

    _watchObject(source, object) {
        if (!object || this._objects.has(object)) return;
        this._objects.set(object, source);

        let signalIds = [];
        try {
            signalIds = GObject.signal_list_ids(object.constructor.$gtype);
        } catch (error) {
            console.warn(`gnoblin events: could not list ${source} signals: ${error.message}`);
        }

        for (const id of signalIds) {
            const query = GObject.signal_query(id);
            if (!query?.signal_name || query.signal_name === "gnoblin-config-event") continue;
            this._watchSignal(source, object, query);
        }

        // GObject's inherited notify signal is not returned by signal_list_ids().
        this._connect(object, "notify", (_object, property) => {
            const name = property?.name ?? "";
            let value;
            try {
                value = name ? object.get_property(name.replaceAll("_", "-")) : undefined;
            } catch {
                value = undefined;
            }
            this._dispatch(source, "notify", object, [value], [property?.value_type]);
        });
    }

    _watchSignal(source, object, query) {
        const signal = canonicalSignalName(query.signal_name);
        const event = `${source}.${signal}`;
        const parameterTypes = query.param_types ?? [];
        if (query.return_type?.name === "void") {
            this._connect(object, signal, (emitter, ...args) => {
                this._dispatch(source, signal, emitter, args, parameterTypes);
            });
            return;
        }

        // An emission hook observes the signal without becoming a handler or
        // changing its return value, which can control Mutter's behavior.
        const signalId = query.signal_id;
        if (this._hooks.has(signalId)) return;
        try {
            const hook = GObject.signal_add_emission_hook(signalId, 0, (_hint, values) => {
                const emitter = values[0];
                const hookedSource = this._objects.get(emitter);
                if (hookedSource) this._dispatch(hookedSource, signal, emitter, values.slice(1), parameterTypes);
                return true;
            });
            this._hooks.set(signalId, hook);
        } catch (error) {
            console.warn(`gnoblin events: could not observe ${event}: ${error.message}`);
        }
    }

    _dispatch(source, signal, emitter, args, parameterTypes) {
        signal = canonicalSignalName(signal);
        if (source === "mutter.workspace-manager" && signal !== "workspace-removed") this._refreshWorkspaceSnapshots();
        const event = `${source}.${signal}`;
        if (!this._config.wantsEvent(event)) return;
        let workspace = null;
        if (source === "mutter.workspace") workspace = emitter;
        else if (source === "mutter.workspace-manager" && Number.isInteger(args[0])) {
            workspace =
                signal === "workspace-removed"
                    ? this._workspaceOrder[args[0]]
                    : global.workspace_manager.get_workspace_by_index(args[0]);
        }
        this._config.dispatchEvent(
            event,
            describeEvent(
                emitter,
                source,
                signal,
                args,
                parameterTypes,
                workspace,
                workspace ? this._workspaceSnapshots.get(workspace) : null,
            ),
        );
    }

    _unwatchObject(object, tracked) {
        if (!tracked.delete(object)) return;
        this._objects.delete(object);
        for (let index = this._connections.length - 1; index >= 0; index--) {
            const [emitter, id] = this._connections[index];
            if (emitter !== object) continue;
            try {
                emitter.disconnect(id);
            } catch {
                // The emitter can be partway through destruction.
            }
            this._connections.splice(index, 1);
        }
    }

    destroy() {
        for (const [object, id] of this._connections) {
            try {
                object.disconnect(id);
            } catch {
                // The emitter may already have been destroyed during shutdown.
            }
        }
        this._connections = [];
        for (const [signalId, hookId] of this._hooks) {
            try {
                GObject.signal_remove_emission_hook(signalId, hookId);
            } catch {
                // The signal type may already be unloaded during shutdown.
            }
        }
        this._hooks.clear();
        this._objects.clear();
        this._windows.clear();
        this._workspaces.clear();
        this._workspaceOrder = [];
        this._workspaceSnapshots.clear();
    }
}
