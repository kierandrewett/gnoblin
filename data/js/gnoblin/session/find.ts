// `gnoblin.find`: the live objects of one kind that match a table of field values or a predicate.

import { camel } from "gnoblin/session/names";

/** A value a `gnoblin.find` table can match. */
export type Scalar = string | number | boolean;

/** The objects `gnoblin.find` can search, as the shell lists them. */
export interface FindSource {
    windows(): object[];
    workspaces(): object[];
    /** The live outputs, or null when this shell has none. */
    outputs(): object[] | null;
}

/** The scalar fields of each kind: what a table filter may name. */
const FIELDS: Record<string, readonly string[]> = {
    Window: [
        "id",
        "appId",
        "title",
        "alive",
        "focused",
        "fullscreen",
        "secure",
        "unresponsive",
        "floating",
        "maximised",
        "minimised",
        "tiled",
        "visible",
        "x",
        "y",
        "width",
        "height",
        "opacity",
        "z",
    ],
    Workspace: ["id", "name", "alive", "index", "active"],
};

/** The kinds `find` accepts. `Output` is listed only by a shell that has outputs. */
export function findKinds(source: FindSource): string[] {
    return source.outputs() === null ? ["Window", "Workspace"] : ["Window", "Workspace", "Output"];
}

/**
 * The live objects of one kind that match a filter, in list order. A table filter keeps the objects whose scalar
 * fields equal every value given (`appId` and `app_id` are one field); a function filter keeps those it returns a
 * truthy value for. An unknown kind or field, or a value that is not a string, number or boolean, throws and names
 * what works.
 */
export function find(source: FindSource, kind: string, filter?: Record<string, Scalar> | ((object: never) => unknown)) {
    const kinds = findKinds(source);
    if (!kinds.includes(kind)) {
        throw new TypeError(`gnoblin.find: no kind named ${JSON.stringify(kind)}; the kinds are ${kinds.join(", ")}`);
    }
    const objects =
        kind === "Window" ? source.windows() : kind === "Workspace" ? source.workspaces() : (source.outputs() ?? []);
    if (filter === undefined) {
        return [...objects];
    }
    if (typeof filter === "function") {
        return objects.filter((object) => filter(object as never));
    }
    if (typeof filter !== "object" || filter === null || Array.isArray(filter)) {
        throw new TypeError("gnoblin.find: the filter is a table of field values or a predicate function");
    }
    const wanted = Object.entries(filter).map(([key, value]) => {
        const field = camel(key);
        const fields = FIELDS[kind] ?? scalarFields(objects[0]);
        if (!fields.includes(field)) {
            throw new TypeError(
                `gnoblin.find: ${kind} has no scalar field ${JSON.stringify(key)}; the fields are ${fields.join(", ")}`,
            );
        }
        if (typeof value !== "string" && typeof value !== "number" && typeof value !== "boolean") {
            throw new TypeError(`gnoblin.find: the value of ${key} must be a string, a number or a boolean`);
        }
        return [field, value] as const;
    });
    return objects.filter((object) => wanted.every(([field, value]) => Reflect.get(object, field) === value));
}

/** The names of an object's scalar properties, for a kind that declares none (the outputs of another module). */
function scalarFields(sample: object | undefined): string[] {
    if (sample === undefined) {
        return [];
    }
    const names = new Set<string>();
    for (let prototype: object | null = sample; prototype !== null && prototype !== Object.prototype;) {
        for (const name of Object.getOwnPropertyNames(prototype)) {
            const value = Object.getOwnPropertyDescriptor(prototype, name)?.get ? Reflect.get(sample, name) : undefined;
            if (["string", "number", "boolean"].includes(typeof value)) {
                names.add(name);
            }
        }
        prototype = Object.getPrototypeOf(prototype) as object | null;
    }
    return [...names];
}
