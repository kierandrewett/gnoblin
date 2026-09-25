// Stable workspace identities for Gnoblin's shell and compositor interfaces.
// MetaWorkspace objects survive reordering, so identities follow the object
// rather than its current one-based position. IDs generated for unconfigured
// workspaces are deliberately session-scoped.
import Meta from "gi://Meta";

const WORKSPACE_ID = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
// Lua workspace selectors accept this generated form at runtime. Config and
// saved-rule validation intentionally accept only WORKSPACE_ID.
const SESSION_WORKSPACE_ID = /^@session-[1-9][0-9]*$/;

export class WorkspaceRegistry {
    constructor(manager, getWorkspaceName = null) {
        this.manager = manager;
        this.getWorkspaceName = getWorkspaceName;
        this.setWorkspaceName = null;
        this.configuredIds = [];
        this.declarations = [];
        this.persistent = new WeakSet();
        this.workspaceNames = new WeakMap();
        this.configuredSlots = new WeakMap();
        this.objectIds = new WeakMap();
        this.idObjects = new Map();
        this.nextSessionId = 1;
        this.pendingCreateId = null;
        this.eventDispatcher = null;

        this._activeWorkspaceSignal = manager.connect("active-workspace-changed", () => {
            const workspace = manager.get_workspace_by_index(manager.get_active_workspace_index());
            if (workspace) this._emit("gnoblin.workspace.activated", this.describe(workspace));
        });
    }

    configure(declarations = [], getWorkspaceName = this.getWorkspaceName, setWorkspaceName = null) {
        if (!Array.isArray(declarations)) throw new Error("workspaces must be an array");
        const normalized = declarations.map((declaration, index) => {
            // Keep accepting the former internal ID-only representation while
            // config and shell resources transition to workspace objects.
            const value =
                typeof declaration === "string"
                    ? { id: declaration, name: getWorkspaceName?.(index) ?? `Workspace ${index + 1}` }
                    : declaration;
            if (!value || typeof value !== "object" || Array.isArray(value))
                throw new Error(`workspace ${index + 1} must be an object with id and name`);
            if (typeof value.id !== "string" || !WORKSPACE_ID.test(value.id))
                throw new Error(`workspace ${index + 1} has an invalid id`);
            if (typeof value.name !== "string" || !value.name.trim())
                throw new Error(`workspace ${index + 1} has an invalid name`);
            return { id: value.id, name: value.name };
        });
        const seen = new Set();
        for (const { id } of normalized) {
            if (seen.has(id)) throw new Error(`duplicate workspace ID: ${id}`);
            seen.add(id);
        }
        const nextIds = normalized.map(({ id }) => id);
        const workspaces = this.workspaces();
        let nextSessionId = this.nextSessionId;
        const sessionId = () => {
            let id;
            do id = `@session-${nextSessionId++}`;
            while (this.idObjects.has(id) || nextIds.includes(id));
            return id;
        };
        const assignments = workspaces.flatMap((workspace, index) => {
            const slot = this.configuredSlots.get(workspace);
            const currentId = this.objectIds.get(workspace);
            if (slot !== undefined) {
                const id = nextIds[slot] ?? sessionId();
                return id !== currentId ? [{ workspace, id, currentId, slot: nextIds[slot] ? slot : undefined }] : [];
            }
            const id = nextIds[index];
            return id && (!currentId || SESSION_WORKSPACE_ID.test(currentId))
                ? [{ workspace, id, currentId, slot: index }]
                : [];
        });
        const changing = new Set(assignments.map(({ workspace }) => workspace));
        const liveWorkspaces = new Set(workspaces);
        const targetIds = new Set();
        for (const { workspace, id } of assignments) {
            if (targetIds.has(id)) throw new Error(`workspace ID is already in use: ${id}`);
            targetIds.add(id);
            const owner = this.idObjects.get(id);
            if (owner && owner !== workspace && liveWorkspaces.has(owner) && !changing.has(owner))
                throw new Error(`workspace ID is already in use: ${id}`);
        }
        for (const { workspace, currentId } of assignments) {
            if (this.idObjects.get(currentId) === workspace) this.idObjects.delete(currentId);
        }
        for (const { workspace, id, slot } of assignments) {
            this.objectIds.set(workspace, id);
            this.idObjects.set(id, workspace);
            if (slot === undefined) this.configuredSlots.delete(workspace);
            else this.configuredSlots.set(workspace, slot);
        }
        this.nextSessionId = nextSessionId;
        this.configuredIds = nextIds;
        this.declarations = normalized;
        this.getWorkspaceName = getWorkspaceName;
        this.setWorkspaceName = setWorkspaceName ?? ((index, name) => Meta.prefs_change_workspace_name(index, name));
        this.sync();
        this.persistent = new WeakSet();
        for (const declaration of normalized) {
            const workspace = this.idObjects.get(declaration.id);
            if (!workspace) continue;
            this.persistent.add(workspace);
            this.workspaceNames.set(workspace, declaration.name);
            this.setWorkspaceName(workspace.index(), declaration.name);
        }
    }

    workspaces() {
        return Array.from({ length: this.manager.n_workspaces }, (_, index) =>
            this.manager.get_workspace_by_index(index),
        );
    }

    sync() {
        for (const [id, workspace] of this.idObjects) {
            if (!this.workspaces().includes(workspace)) this.idObjects.delete(id);
        }
        for (const [index, workspace] of this.workspaces().entries()) {
            if (this.objectIds.has(workspace)) continue;
            const configuredId = this.configuredIds[index];
            if (configuredId && !this.idObjects.has(configuredId)) {
                this.objectIds.set(workspace, configuredId);
                this.idObjects.set(configuredId, workspace);
                this.configuredSlots.set(workspace, index);
                const declaration = this.declarations[index];
                if (declaration) {
                    this.persistent.add(workspace);
                    this.workspaceNames.set(workspace, declaration.name);
                    this.setWorkspaceName?.(workspace.index(), declaration.name);
                }
                continue;
            }
            let id = this.pendingCreateId;
            if (id) {
                this.pendingCreateId = null;
            } else {
                do id = `@session-${this.nextSessionId++}`;
                while (this.idObjects.has(id) || this.configuredIds.includes(id));
            }
            this.objectIds.set(workspace, id);
            this.idObjects.set(id, workspace);
        }
    }

    getId(workspace) {
        this.sync();
        if (!workspace || !this.objectIds.has(workspace)) throw new Error("workspace is no longer available");
        return this.objectIds.get(workspace);
    }

    resolve(selector) {
        this.sync();
        if (!selector || typeof selector !== "object") throw new Error("workspace selector must specify id or number");
        const hasId = Object.hasOwn(selector, "id");
        const hasNumber = Object.hasOwn(selector, "number");
        if (hasId === hasNumber) throw new Error("workspace selector must specify exactly one of id or number");
        let workspace;
        if (hasId) {
            if (typeof selector.id !== "string" || !selector.id)
                throw new Error("workspace id must be a nonempty string");
            workspace = this.idObjects.get(selector.id);
        } else {
            if (!Number.isInteger(selector.number) || selector.number < 1)
                throw new Error("workspace number must be a positive integer");
            workspace = this.manager.get_workspace_by_index(selector.number - 1);
        }
        if (!workspace || !this.workspaces().includes(workspace))
            throw new Error("workspace not found; list workspaces first");
        return workspace;
    }

    describe(workspace) {
        const number = this.workspaces().indexOf(workspace) + 1;
        if (!number) throw new Error("workspace is no longer available");
        const id = this.getId(workspace);
        return {
            id,
            number,
            name: this.workspaceNames.get(workspace) || this.getWorkspaceName?.(number - 1) || `Workspace ${number}`,
            active: this.manager.get_active_workspace_index() === number - 1,
            windows: workspace.list_windows().length,
            persistent: this.persistent.has(workspace),
        };
    }

    list() {
        return this.workspaces().map((workspace) => this.describe(workspace));
    }

    setEventDispatcher(dispatcher) {
        if (dispatcher !== null && typeof dispatcher !== "function")
            throw new TypeError("event dispatcher must be a function or null");
        this.eventDispatcher = dispatcher;
    }

    _emit(name, payload) {
        try {
            this.eventDispatcher?.(name, payload);
        } catch (error) {
            console.warn(`gnoblin workspace event ${name}: ${error.message}`);
        }
    }

    create({ id = null, name, activate = false } = {}) {
        this.sync();
        if (id !== null && (typeof id !== "string" || !WORKSPACE_ID.test(id)))
            throw new Error("workspace id must be 1 to 64 letters, numbers, dots, underscores, or hyphens");
        if (id && this.idObjects.has(id)) throw new Error(`workspace ID is already in use: ${id}`);
        if (typeof name !== "string" || !name.trim()) throw new Error("workspace name must be a nonempty string");
        if (typeof activate !== "boolean") throw new Error("activate must be a boolean");

        const actualId = id ?? this._newSessionId();
        this.pendingCreateId = actualId;
        let workspace;
        try {
            workspace = this.manager.append_new_workspace(false, global.get_current_time());
        } finally {
            this.pendingCreateId = null;
        }
        if (!workspace) throw new Error("could not create workspace");
        const assignedId = this.objectIds.get(workspace);
        if (assignedId && assignedId !== actualId && this.idObjects.get(assignedId) === workspace)
            this.idObjects.delete(assignedId);
        this.objectIds.set(workspace, actualId);
        this.idObjects.set(actualId, workspace);
        this.workspaceNames.set(workspace, name);
        Meta.prefs_change_workspace_name(workspace.index(), name);
        const record = this.describe(workspace);
        this._emit("gnoblin.workspace.created", record);
        if (activate) workspace.activate(global.get_current_time());
        return record;
    }

    rename(selector, name) {
        if (typeof name !== "string" || !name.trim()) throw new Error("workspace name must be a nonempty string");
        const workspace = this.resolve(selector);
        const number = workspace.index() + 1;
        Meta.prefs_change_workspace_name(number - 1, name);
        this.workspaceNames.set(workspace, name);
        const record = this.describe(workspace);
        this._emit("gnoblin.workspace.renamed", record);
        return record;
    }

    remove(selector) {
        const workspace = this.resolve(selector);
        const record = this.describe(workspace);
        if (record.persistent) throw new Error("cannot remove a declared persistent workspace");
        if (record.active) throw new Error("cannot remove the active workspace; switch first");
        if (record.windows > 0) throw new Error("cannot remove a workspace that still has windows");
        if (this.manager.n_workspaces <= 1) throw new Error("cannot remove the only workspace");

        this.manager.remove_workspace(workspace, global.get_current_time());
        this.sync();
        for (const remaining of this.workspaces()) {
            const name = this.workspaceNames.get(remaining);
            Meta.prefs_change_workspace_name(remaining.index(), name ?? "");
        }
        this._emit("gnoblin.workspace.removed", record);
        return { ...record, active: false, number: record.number };
    }

    switch(selector) {
        const workspace = this.resolve(selector);
        workspace.activate(global.get_current_time());
        return this.describe(workspace);
    }

    next() {
        return this._switchRelative(1);
    }

    previous() {
        return this._switchRelative(-1);
    }

    _switchRelative(delta) {
        const count = this.manager.n_workspaces;
        if (!count) throw new Error("no workspaces are available");
        const current = this.manager.get_active_workspace_index();
        return this.switch({ number: ((current + delta + count) % count) + 1 });
    }

    moveWindow(windowSelector, selector, follow = false) {
        if (typeof follow !== "boolean") throw new Error("follow must be a boolean");
        let window;
        if (windowSelector === "active") {
            window = global.display.focus_window;
        } else {
            const id = String(windowSelector);
            window = global.display
                .list_all_windows()
                .find((candidate) => String(candidate.get_stable_sequence()) === id);
        }
        if (!window) throw new Error("window not found");
        const workspace = this.resolve(selector);
        window.change_workspace(workspace);
        if (follow) workspace.activate(global.get_current_time());
        return { workspace: this.describe(workspace), window: String(window.get_stable_sequence()), follow };
    }

    dispatch(method, arguments_ = {}) {
        if (typeof method !== "string") throw new Error("workspace operation method must be a string");
        if (!arguments_ || typeof arguments_ !== "object" || Array.isArray(arguments_))
            throw new Error("workspace operation arguments must be an object");
        const name = method.startsWith("workspace.") ? method.slice("workspace.".length) : method;
        switch (name) {
            case "list":
                return this.list();
            case "create":
                return this.create(arguments_);
            case "rename": {
                const { id, number, name: workspaceName } = arguments_;
                return this.rename(this._selector({ id, number }), workspaceName);
            }
            case "remove": {
                const { id, number } = arguments_;
                return this.remove(this._selector({ id, number }));
            }
            case "switch": {
                const { id, number } = arguments_;
                return this.switch(this._selector({ id, number }));
            }
            case "next":
                return this.next();
            case "previous":
                return this.previous();
            case "move_window":
            case "moveWindow": {
                const { window, workspace, follow = false } = arguments_;
                return this.moveWindow(window, workspace, follow);
            }
            case "move_active": {
                const { workspace, follow = false } = arguments_;
                return this.moveWindow("active", workspace, follow);
            }
            default:
                throw new Error(`unknown workspace operation: ${method}`);
        }
    }

    _selector(selector) {
        const hasId = selector.id !== undefined && selector.id !== null;
        const hasNumber = selector.number !== undefined && selector.number !== null;
        if (hasId === hasNumber) throw new Error("workspace selector must specify exactly one of id or number");
        return hasId ? { id: selector.id } : { number: selector.number };
    }

    _newSessionId() {
        let id;
        do id = `@session-${this.nextSessionId++}`;
        while (this.idObjects.has(id) || this.configuredIds.includes(id));
        return id;
    }
}

let registry = null;

function currentRegistry() {
    if (!registry) registry = new WorkspaceRegistry(global.workspace_manager);
    return registry;
}

export function configure(declarations = [], getWorkspaceName, setWorkspaceName) {
    currentRegistry().configure(declarations, getWorkspaceName, setWorkspaceName);
}

export function setEventDispatcher(dispatcher) {
    currentRegistry().setEventDispatcher(dispatcher);
}

export function dispatch(method, arguments_) {
    return currentRegistry().dispatch(method, arguments_);
}

export function getId(workspace) {
    return currentRegistry().getId(workspace);
}

export function resolve(selector) {
    return currentRegistry().resolve(selector);
}

export function describe(workspace) {
    return currentRegistry().describe(workspace);
}

export function list() {
    return currentRegistry().list();
}
