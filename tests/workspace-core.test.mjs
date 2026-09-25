import assert from "node:assert/strict";
import { WorkspaceRegistry } from "../src/gnome-shell-overlay/js/ui/components/gnoblinWorkspaces.js";

class Workspace {
    list_windows() {
        return [];
    }
}

class Manager {
    constructor(workspaces) {
        this.workspaces = workspaces;
        this.active = workspaces[0];
    }
    get n_workspaces() {
        return this.workspaces.length;
    }
    get_workspace_by_index(index) {
        return this.workspaces[index] ?? null;
    }
    get_active_workspace() {
        return this.active;
    }
    get_active_workspace_index() {
        return this.workspaces.indexOf(this.active);
    }
}

const first = new Workspace();
const second = new Workspace();
const manager = new Manager([first, second]);
const workspaceNames = ["Configured Code", "Configured Chat"];
const registry = new WorkspaceRegistry(manager, (index) => workspaceNames[index]);
registry.configure(["code", "chat"]);

assert.deepEqual(registry.list(), [
    { id: "code", number: 1, name: "Configured Code", active: true, windows: 0 },
    { id: "chat", number: 2, name: "Configured Chat", active: false, windows: 0 },
]);
workspaceNames[0] = "Renamed Code";
assert.equal(registry.describe(first).name, "Renamed Code");
manager.workspaces = [second, first];
assert.equal(registry.getId(first), "code");
assert.equal(registry.resolve({ id: "code" }), first);
assert.equal(registry.resolve({ number: 1 }), second);
assert.equal(registry.describe(first).number, 2);

const transient = new Workspace("Temporary");
manager.workspaces.push(transient);
const generated = registry.getId(transient);
assert.match(generated, /^@session-\d+$/);
assert.equal(registry.resolve({ id: generated }), transient);
assert.throws(() => registry.resolve({ id: "missing" }), /workspace not found/);
assert.throws(() => registry.resolve({ id: "code", number: 1 }), /exactly one/);
assert.throws(() => registry.configure(["same", "same"]), /duplicate/);

const early = new Workspace("Early");
const late = new Workspace("Late");
const lateManager = new Manager([early, late]);
const lateRegistry = new WorkspaceRegistry(lateManager);
assert.match(lateRegistry.getId(early), /^@session-/);
lateRegistry.configure(["early", "late"]);
assert.equal(lateRegistry.getId(early), "early");
lateManager.workspaces = [late, early];
lateRegistry.configure(["early-renamed", "late-renamed"]);
assert.equal(lateRegistry.getId(early), "early-renamed");
assert.equal(lateRegistry.getId(late), "late-renamed");

const configured = new Workspace("Configured");
const ephemeral = new Workspace("Ephemeral");
const collisionManager = new Manager([configured, ephemeral]);
const collisionRegistry = new WorkspaceRegistry(collisionManager);
collisionRegistry.configure(["configured"]);
const ephemeralId = collisionRegistry.getId(ephemeral);
assert.match(ephemeralId, /^@session-\d+$/);
collisionRegistry.configure(["session-1"]);
assert.equal(collisionRegistry.getId(configured), "session-1");
assert.equal(collisionRegistry.getId(ephemeral), ephemeralId);
assert.throws(() => collisionRegistry.configure(["same", "same"]), /duplicate/);
assert.equal(collisionRegistry.getId(configured), "session-1");
collisionRegistry.configure([]);
assert.notEqual(collisionRegistry.getId(configured), "session-1");
assert.equal(collisionRegistry.getId(ephemeral), ephemeralId);

const reservedManager = new Manager([new Workspace("One"), new Workspace("Two")]);
const reservedRegistry = new WorkspaceRegistry(reservedManager);
reservedRegistry.getId(reservedManager.workspaces[0]);
assert.throws(() => reservedRegistry.configure(["@session-1"]), /workspace IDs must/);
reservedRegistry.configure(["session-1"]);
assert.equal(reservedRegistry.getId(reservedManager.workspaces[0]), "session-1");
assert.match(reservedRegistry.getId(reservedManager.workspaces[1]), /^@session-/);

console.log("PASS workspace identity and selector behavior");
