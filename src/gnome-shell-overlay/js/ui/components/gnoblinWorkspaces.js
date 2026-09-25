// Stable workspace identities for Gnoblin's shell and compositor interfaces.
// MetaWorkspace objects survive reordering, so identities follow the object
// rather than its current one-based position. IDs generated for unconfigured
// workspaces are deliberately session-scoped.
export class WorkspaceRegistry {
    constructor(manager, getWorkspaceName = null) {
        this.manager = manager;
        this.getWorkspaceName = getWorkspaceName;
        this.configuredIds = [];
        this.configuredSlots = new WeakMap();
        this.objectIds = new WeakMap();
        this.idObjects = new Map();
        this.nextSessionId = 1;
    }

    configure(ids = [], getWorkspaceName = this.getWorkspaceName) {
        if (!Array.isArray(ids)) throw new Error("workspace-ids must be an array");
        const seen = new Set();
        for (const id of ids) {
            if (typeof id !== "string" || !/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(id))
                throw new Error("workspace IDs must be 1 to 64 letters, numbers, dots, underscores, or hyphens");
            if (seen.has(id)) throw new Error(`duplicate workspace ID: ${id}`);
            seen.add(id);
        }
        const nextIds = [...ids];
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
            return id && (!currentId || currentId.startsWith("@session-"))
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
        this.getWorkspaceName = getWorkspaceName;
        this.sync();
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
                continue;
            }
            let id;
            do id = `@session-${this.nextSessionId++}`;
            while (this.idObjects.has(id) || this.configuredIds.includes(id));
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
        return {
            id: this.getId(workspace),
            number,
            name: this.getWorkspaceName?.(number - 1) || `Workspace ${number}`,
            active: this.manager.get_active_workspace_index() === number - 1,
            windows: workspace.list_windows().length,
        };
    }

    list() {
        return this.workspaces().map((workspace) => this.describe(workspace));
    }
}

let registry = null;

function currentRegistry() {
    if (!registry) registry = new WorkspaceRegistry(global.workspace_manager);
    return registry;
}

export function configure(ids = [], getWorkspaceName) {
    currentRegistry().configure(ids, getWorkspaceName);
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
