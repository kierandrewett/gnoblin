// Workspace sets, from data/retired shell/gnoblin/shell/sets.retired shell: a set is `{ name, entries: [{ workspace, output? }] }`, each
// entry naming a workspace and the output it should show on. `defineSets` and `activateSet` are pure functions over
// plain data and return the retired module's results; `WorkspaceSets` keeps the list in force and applies a plan.
//
// A refused list is never partly applied: the previous list stays. An entry whose output is missing falls back to
// the primary output with a warning, as in the retired shell.

export interface SetEntry {
    /** A workspace id. */
    workspace: string;
    /** The output the workspace shows on; none means the primary one. */
    output?: string;
}

export interface WorkspaceSet {
    name: string;
    entries: SetEntry[];
}

export type SetError =
    | { kind: "empty_name" }
    | { kind: "duplicate_name"; name: string }
    | { kind: "empty_set"; name: string }
    | { kind: "undeclared_workspace"; set: string; workspace: string }
    | { kind: "duplicate_output"; set: string; output: string }
    | { kind: "unknown_set"; name: string };

export type SetWarning = { kind: "output_fallback"; set: string; output: string };

export interface SetPlacement {
    workspace: string;
    output: string;
}

export interface SetPlan {
    set: string;
    placements: SetPlacement[];
    warnings: SetWarning[];
    /** False when the set is the one already live. */
    changed: boolean;
}

export type SetResult<T> = { ok: true; value: T } | { ok: false; error: SetError };

function copy(list: readonly WorkspaceSet[]): WorkspaceSet[] {
    return list.map((set) => ({
        name: set.name,
        entries: set.entries.map((entry) => ({
            workspace: entry.workspace,
            ...(entry.output === undefined ? {} : { output: entry.output }),
        })),
    }));
}

/** Validates `incoming` against the declared workspace ids. On success the value is a copy of it. */
export function defineSets(
    incoming: readonly WorkspaceSet[],
    declared: ReadonlySet<string>,
): SetResult<WorkspaceSet[]> {
    const names = new Set<string>();
    for (const set of incoming) {
        if (set.name === "") {
            return { ok: false, error: { kind: "empty_name" } };
        }
        if (names.has(set.name)) {
            return { ok: false, error: { kind: "duplicate_name", name: set.name } };
        }
        names.add(set.name);
        if (set.entries.length === 0) {
            return { ok: false, error: { kind: "empty_set", name: set.name } };
        }
        const outputs = new Set<string>();
        for (const entry of set.entries) {
            if (!declared.has(entry.workspace)) {
                return {
                    ok: false,
                    error: { kind: "undeclared_workspace", set: set.name, workspace: entry.workspace },
                };
            }
            if (entry.output !== undefined) {
                if (outputs.has(entry.output)) {
                    return { ok: false, error: { kind: "duplicate_output", set: set.name, output: entry.output } };
                }
                outputs.add(entry.output);
            }
        }
    }
    return { ok: true, value: copy(incoming) };
}

export interface ActivateOptions {
    /** The workspace ids that exist. */
    declared: ReadonlySet<string>;
    /** The names of the outputs that are connected. */
    outputs: readonly string[];
    /** The fallback output's name. */
    primary: string;
    /** The name of the set now live, if any. */
    active?: string | null;
}

/** The plan that makes the set `name` live. */
export function activateSet(list: readonly WorkspaceSet[], name: string, options: ActivateOptions): SetResult<SetPlan> {
    let set: WorkspaceSet | undefined;
    for (const candidate of list) {
        if (candidate.name === name) {
            set = candidate; // the last of the name, as the retired module reads it
        }
    }
    if (set === undefined) {
        return { ok: false, error: { kind: "unknown_set", name } };
    }
    const shown = new Set(options.outputs);
    const placements: SetPlacement[] = [];
    const warnings: SetWarning[] = [];
    for (const entry of set.entries) {
        if (!options.declared.has(entry.workspace)) {
            return { ok: false, error: { kind: "undeclared_workspace", set: name, workspace: entry.workspace } };
        }
        let output = options.primary;
        if (entry.output !== undefined && shown.has(entry.output)) {
            output = entry.output;
        } else if (entry.output !== undefined) {
            warnings.push({ kind: "output_fallback", set: name, output: entry.output });
        }
        placements.push({ workspace: entry.workspace, output });
    }
    return {
        ok: true,
        value: { set: name, placements, warnings, changed: (options.active ?? null) !== name },
    };
}

/** The words the stock shell reports a refusal in. */
export function explainSetError(error: SetError): string {
    switch (error.kind) {
        case "empty_name":
            return "a set has an empty name";
        case "duplicate_name":
            return `two sets are named ${JSON.stringify(error.name)}`;
        case "empty_set":
            return `set ${JSON.stringify(error.name)} has no entries`;
        case "undeclared_workspace":
            return `set ${JSON.stringify(error.set)} names workspace ${JSON.stringify(error.workspace)}, which no workspace has`;
        case "duplicate_output":
            return `set ${JSON.stringify(error.set)} has two entries for output ${JSON.stringify(error.output)}`;
        case "unknown_set":
            return `no set is named ${JSON.stringify(error.name)}`;
    }
}

/** A refused `define` or `activate`: `error` is the cause as data, the message the words for it. */
export class WorkspaceSetError extends Error {
    readonly error: SetError;

    constructor(error: SetError) {
        super(explainSetError(error));
        this.name = "WorkspaceSetError";
        this.error = error;
    }
}

/** What `WorkspaceSets` needs of the shell. */
export interface SetsEnvironment {
    declared(): ReadonlySet<string>;
    /** The connected outputs and the primary one. */
    outputs(): { names: readonly string[]; primary: string };
    /** Shows `workspace` on the output named `output`; left out, on the one the shell would show it on. */
    show(workspace: string, output?: string): void;
    /** A condition the user should hear of that does not stop the activation. */
    report(message: string): void;
}

/** `gnoblin.workspaces.sets`: the sets in force and the one that is live. */
export class WorkspaceSets {
    readonly #environment: SetsEnvironment;
    #list: WorkspaceSet[] = [];
    #active: string | null = null;

    constructor(environment: SetsEnvironment) {
        this.#environment = environment;
    }

    /** A copy of the sets in force. */
    get list(): WorkspaceSet[] {
        return copy(this.#list);
    }

    /** The name of the live set, or null. */
    get active(): string | null {
        return this.#active;
    }

    /**
     * Replaces the sets. A list that does not check (an empty or repeated name, a set with no entries, a workspace
     * that does not exist, two entries for one output) throws `WorkspaceSetError` and leaves the sets in force.
     */
    define(list: readonly WorkspaceSet[]): void {
        const result = defineSets(list, this.#environment.declared());
        if (!result.ok) {
            throw new WorkspaceSetError(result.error);
        }
        this.#list = result.value;
    }

    /**
     * Makes the set `name` live: each entry shows its workspace on its output. An entry whose output is not connected
     * is reported and shows on the primary output. Activating the live set again changes nothing. An unknown set or a
     * workspace that has since gone throws `WorkspaceSetError`.
     */
    activate(name: string): SetPlan {
        const { names, primary } = this.#environment.outputs();
        const result = activateSet(this.#list, name, {
            declared: this.#environment.declared(),
            outputs: names,
            primary,
            active: this.#active,
        });
        if (!result.ok) {
            throw new WorkspaceSetError(result.error);
        }
        const plan = result.value;
        for (const warning of plan.warnings) {
            this.#environment.report(
                `workspace set ${JSON.stringify(warning.set)}: output ${JSON.stringify(warning.output)} is not connected; its workspace shows on the primary output`,
            );
        }
        if (!plan.changed) {
            return plan;
        }
        this.#active = name;
        for (const placement of plan.placements) {
            this.#environment.show(placement.workspace, placement.output);
        }
        return plan;
    }
}
