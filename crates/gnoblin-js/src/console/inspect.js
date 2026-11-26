// What the console shows of a value on a generation that runs the native library, and `inspect`, the console's helpers.
//
// The script evaluates to `{ render, plain, clients, workspaces, bindings }`; the console installs it as the global `inspect` (when the
// prompt has not already defined one) and renders every reply with its `render`. It is the native library's version of the TypeScript
// library's `inspect` (`data/js/gnoblin-core/inspect.ts`): the reply is `{ type, value?, text, truncated }`, a cycle prints as
// `[Circular]`, a function as `[Function name]`, and a rendering that outgrows its cap is cut and says so.
//
// The native library's objects are generated classes: a library object is an instance whose members are getters on its class's
// prototype chain (a value class holds its fields on the instance). So one describer serves them all and no kind is registered:
//
// - a class name, and `#id` when the object has an `id`;
// - as the value itself: every readable getter of the chain, in the order the class declares them (a getter that throws shows
//   `[Error: message]`);
// - nested in another value, a *brief*: the id and the first two string members (`Client#3 { appId: "foot", title: "term" }`);
// - an object with a `toList` and a `length` is a collection and prints as `Name(n) [ ...briefs ]`.
(function () {
    "use strict";

    const DEFAULT_MAX_BYTES = 65536;
    const DEFAULT_MAX_DEPTH = 6;
    // A string longer than this is cut: a console is not a file viewer.
    const MAX_STRING = 4096;
    // How many string members a brief shows after the id.
    const BRIEF_STRINGS = 2;

    /** How many bytes `text` takes in UTF-8, which is what the reply's limit counts (a lone surrogate travels as U+FFFD: three). */
    function byteLength(text) {
        let bytes = 0;
        for (let at = 0; at < text.length; at += 1) {
            const unit = text.charCodeAt(at);
            if (unit < 0x80) {
                bytes += 1;
            } else if (unit < 0x800) {
                bytes += 2;
            } else if (
                unit >= 0xd800 &&
                unit < 0xdc00 &&
                at + 1 < text.length &&
                (text.charCodeAt(at + 1) & 0xfc00) === 0xdc00
            ) {
                // A surrogate pair is one character of four bytes.
                bytes += 4;
                at += 1;
            } else {
                bytes += 3;
            }
        }
        return bytes;
    }

    /** The longest start of `text` that takes at most `max` bytes and ends between two characters. */
    function cutToBytes(text, max) {
        let bytes = 0;
        let at = 0;
        while (at < text.length) {
            const unit = text.charCodeAt(at);
            const pair =
                unit >= 0xd800 &&
                unit < 0xdc00 &&
                at + 1 < text.length &&
                (text.charCodeAt(at + 1) & 0xfc00) === 0xdc00;
            const size = unit < 0x80 ? 1 : unit < 0x800 ? 2 : pair ? 4 : 3;
            if (bytes + size > max) break;
            bytes += size;
            at += pair ? 2 : 1;
        }
        return text.slice(0, at);
    }

    function spend(state, text) {
        state.left -= byteLength(text);
    }

    function leaf(state, data, text) {
        spend(state, text);
        return { data, text };
    }

    function cut(state) {
        state.truncated = true;
        return { data: "[truncated]", text: "[truncated]" };
    }

    function failure(error) {
        let message;
        try {
            message = error instanceof Error ? error.message : String(error);
        } catch (inner) {
            message = "unreadable";
        }
        return "[Error: " + message + "]";
    }

    /** The name of the class an object was made by, or `Object`. An object that will not say throws. */
    function nameOf(value) {
        const ctor = value.constructor;
        const name = typeof ctor === "function" ? ctor.name : "";
        return typeof name === "string" && name !== "" ? name : "Object";
    }

    function describeFunction(value) {
        const source = Function.prototype.toString.call(value);
        const name = value.name === "" ? "(anonymous)" : value.name;
        return source.startsWith("class") ? "[class " + name + "]" : "[Function " + name + "]";
    }

    /** The names of the getters an object's class chain declares, nearest class first, each once. */
    function getterNames(value) {
        const names = [];
        const seen = new Set();
        for (
            let at = Object.getPrototypeOf(value);
            at !== null && at !== Object.prototype;
            at = Object.getPrototypeOf(at)
        ) {
            for (const key of Object.getOwnPropertyNames(at)) {
                if (key === "constructor" || seen.has(key)) continue;
                seen.add(key);
                const descriptor = Object.getOwnPropertyDescriptor(at, key);
                if (descriptor !== undefined && typeof descriptor.get === "function") names.push(key);
            }
        }
        return names;
    }

    /** Whether `value` is made of getters (a library object) rather than of fields of its own. */
    function isAccessorBacked(value) {
        return Object.getPrototypeOf(value) !== null && getterNames(value).length > 0;
    }

    /**
     * The members of a library collection, or `undefined` when the object is not one. The core's collections list themselves with
     * `toList()` and the std library's with `members`; both have a `length`.
     */
    function collectionMembers(value) {
        if (typeof value.length !== "number") return undefined;
        if (typeof value.toList === "function") return value.toList();
        const members = value.members;
        return Array.isArray(members) ? members : undefined;
    }

    function walk(value, state, depth) {
        if (state.left <= 0) return cut(state);
        switch (typeof value) {
            case "undefined":
                return leaf(state, null, "undefined");
            case "boolean":
                return leaf(state, value, String(value));
            case "number": {
                // JSON has no NaN, Infinity or negative zero: they travel as their names.
                const text = Object.is(value, -0) ? "-0" : String(value);
                return leaf(state, Number.isFinite(value) && text !== "-0" ? value : text, text);
            }
            case "bigint":
                return leaf(state, value + "n", value + "n");
            case "symbol":
                return leaf(state, value.toString(), value.toString());
            case "string": {
                let text = value;
                if (text.length > MAX_STRING) {
                    text = text.slice(0, MAX_STRING);
                    state.truncated = true;
                }
                return leaf(state, text, JSON.stringify(text));
            }
            case "function": {
                const text = describeFunction(value);
                return leaf(state, text, text);
            }
            default:
                break;
        }
        if (value === null) return leaf(state, null, "null");
        return walkObject(value, state, depth);
    }

    function walkObject(value, state, depth) {
        if (state.ancestors.has(value)) return leaf(state, "[Circular]", "[Circular]");
        let name;
        try {
            name = nameOf(value);
        } catch (error) {
            // An object whose class cannot be read is shown as what it says of itself, and has no data form.
            let text;
            try {
                text = String(value);
            } catch (inner) {
                text = "[object]";
            }
            return leaf(state, undefined, text);
        }
        if (depth > state.maxDepth) {
            const text = Array.isArray(value) ? "[Array]" : "[" + name + "]";
            return leaf(state, text, text);
        }
        state.ancestors.add(value);
        try {
            if (Array.isArray(value)) return walkList(value, state, depth, "");
            if (!isBuiltin(value) && isAccessorBacked(value)) return walkHandle(value, name, state, depth);
            return walkPlain(value, name, state, depth);
        } finally {
            state.ancestors.delete(value);
        }
    }

    /** A list of values, each shown as a brief: only the value itself shows everything of its kind. */
    function walkList(items, state, depth, label) {
        const data = [];
        const parts = [];
        spend(state, "[ ]");
        for (let index = 0; index < items.length; index += 1) {
            if (state.left <= 0) {
                state.truncated = true;
                const rest = "... " + (items.length - index) + " more";
                data.push(rest);
                parts.push(rest);
                break;
            }
            let shown;
            try {
                shown = walk(items[index], state, depth + 1);
            } catch (error) {
                const text = failure(error);
                shown = leaf(state, text, text);
            }
            // The separator counts too, or a list of short numbers would cost a third of what it prints.
            spend(state, ", ");
            data.push(shown.data);
            parts.push(shown.text);
        }
        return { data, text: parts.length === 0 ? label + "[]" : label + "[ " + parts.join(", ") + " ]" };
    }

    /** The `key: value` parts of the members `list` names (`[key, read]`), cut when the budget is spent. */
    function entriesOf(list, state, depth) {
        const data = {};
        const parts = [];
        for (let index = 0; index < list.length; index += 1) {
            const [key, read] = list[index];
            if (state.left <= 0) {
                state.truncated = true;
                const rest = "... " + (list.length - index) + " more";
                data["..."] = rest;
                parts.push(rest);
                break;
            }
            let shown;
            try {
                shown = walk(read(), state, depth + 1);
            } catch (error) {
                const text = failure(error);
                shown = leaf(state, text, text);
            }
            spend(state, key + ": , ");
            data[key] = shown.data;
            parts.push((/^[A-Za-z_$][\w$]*$/.test(key) ? key : JSON.stringify(key)) + ": " + shown.text);
        }
        return { data, parts };
    }

    /** The id of a library object, when it has one that reads as text or a number. */
    function idOf(value) {
        try {
            const id = value.id;
            return typeof id === "string" || typeof id === "number" ? String(id) : undefined;
        } catch (error) {
            return undefined;
        }
    }

    /** What a library object shows when it is nested in another value: its id and its first string members. */
    function briefOf(value) {
        const picked = [];
        for (const key of getterNames(value)) {
            if (picked.length >= BRIEF_STRINGS) break;
            if (key === "id") continue;
            try {
                const member = value[key];
                if (typeof member === "string") picked.push([key, () => member]);
            } catch (error) {
                // A member that cannot be read is not a brief's.
            }
        }
        return picked;
    }

    function walkHandle(value, name, state, depth) {
        const items = collectionMembers(value);
        if (items !== undefined) {
            return walkList(items, state, depth, name + "(" + items.length + ") ");
        }
        const id = idOf(value);
        const members =
            depth === 0
                ? getterNames(value)
                      .filter((key) => key !== "id")
                      .map((key) => [key, () => value[key]])
                : briefOf(value);
        const { data, parts } = entriesOf(members, state, depth);
        const head = id === undefined ? name : name + "#" + id;
        spend(state, head);
        return {
            data: Object.assign({ $class: name }, id === undefined ? {} : { id }, data),
            text: parts.length === 0 ? head + " {}" : head + " { " + parts.join(", ") + " }",
        };
    }

    /** Whether the class is one the engine made: it has its own way of printing, and none of its getters are members. */
    function isBuiltin(value) {
        return (
            value instanceof Error ||
            value instanceof Date ||
            value instanceof RegExp ||
            value instanceof Promise ||
            value instanceof Map ||
            value instanceof Set ||
            value instanceof WeakMap ||
            value instanceof WeakSet ||
            value instanceof ArrayBuffer ||
            ArrayBuffer.isView(value)
        );
    }

    function walkPlain(value, name, state, depth) {
        if (value instanceof Error) {
            return leaf(state, { $class: value.name, message: value.message }, value.name + ": " + value.message);
        }
        if (value instanceof Date) {
            const text = Number.isNaN(value.getTime()) ? "Invalid Date" : value.toISOString();
            return leaf(state, text, text);
        }
        if (value instanceof RegExp) return leaf(state, String(value), String(value));
        if (value instanceof Promise) return leaf(state, "[Promise]", "[Promise]");
        if (value instanceof Map) {
            const shown = walkList([...value.entries()], state, depth, "");
            return { data: { $class: "Map", entries: shown.data }, text: "Map(" + value.size + ") " + shown.text };
        }
        if (value instanceof Set) {
            const shown = walkList([...value.values()], state, depth, "");
            return { data: { $class: "Set", values: shown.data }, text: "Set(" + value.size + ") " + shown.text };
        }
        if (ArrayBuffer.isView(value) || value instanceof ArrayBuffer) {
            const text = "[" + name + " " + value.byteLength + "]";
            return leaf(state, text, text);
        }
        if (value instanceof WeakMap || value instanceof WeakSet) {
            const text = "[" + name + "]";
            return leaf(state, text, text);
        }
        const { data, parts } = entriesOf(
            Object.keys(value).map((key) => [key, () => value[key]]),
            state,
            depth,
        );
        const bare = Object.getPrototypeOf(value) === null || name === "Object";
        spend(state, name);
        return {
            data: bare ? data : Object.assign({ $class: name }, data),
            text: bare
                ? parts.length === 0
                    ? "{}"
                    : "{ " + parts.join(", ") + " }"
                : parts.length === 0
                  ? name + " {}"
                  : name + " { " + parts.join(", ") + " }",
        };
    }

    /** How the console names a value's type: `undefined`, `null`, `boolean`, ..., `array`, `function`, `object`, or a class name. */
    function typeOf(value) {
        if (value === null) return "null";
        if (Array.isArray(value)) return "array";
        if (typeof value === "object") {
            try {
                const name = nameOf(value);
                return name === "Object" ? "object" : name;
            } catch (error) {
                // A class that cannot be read leaves the plain type.
                return "object";
            }
        }
        return typeof value;
    }

    function stateFor(options) {
        return {
            left: options.maxBytes === undefined ? DEFAULT_MAX_BYTES : options.maxBytes,
            truncated: false,
            maxDepth: options.maxDepth === undefined ? DEFAULT_MAX_DEPTH : options.maxDepth,
            ancestors: new Set(),
        };
    }

    /** The value as plain JSON-able data: the data half of `render`. */
    function plain(value, options) {
        const data = walk(value, stateFor(options || {}), 0).data;
        return value === undefined ? undefined : data;
    }

    /**
     * Renders `value` for the console and returns the reply as JSON text: `{ type, value?, text, truncated }`. What does not fit in
     * `maxBytes` is left out and `truncated` says so. A value that is one piece bigger than that (a long string) is cut in its text and
     * has no `value`.
     */
    function render(value, options) {
        const max = (options || {}).maxBytes === undefined ? DEFAULT_MAX_BYTES : options.maxBytes;
        const state = stateFor(options || {});
        const shown = walk(value, state, 0);
        const reply = { type: typeOf(value), text: shown.text, truncated: state.truncated };
        if (!state.truncated && byteLength(shown.text) > max) {
            reply.text = cutToBytes(shown.text, max);
            reply.truncated = true;
        } else if (value !== undefined && shown.data !== undefined) {
            reply.value = shown.data;
        }
        return JSON.stringify(reply);
    }

    // -- the helpers a console user types ---------------------------------------------------------------------------

    function live() {
        const found = globalThis.gnoblin;
        if (found === undefined) throw new Error("inspect: there is no `gnoblin` global in this runtime");
        return found;
    }

    /** The members of a collection or a list, as an array. */
    function membersOf(collection) {
        if (collection === undefined || collection === null) return [];
        return collectionMembers(collection) || Array.from(collection);
    }

    /** Every client as plain data. */
    function clients() {
        const gnoblin = live();
        const focused = gnoblin.clients.focused;
        return membersOf(gnoblin.clients).map((client) => {
            const workspace = client.workspace;
            return {
                id: client.id,
                appId: client.appId,
                title: client.title,
                focused: client === focused || client.focused === true,
                fullscreen: client.fullscreen,
                floating: client.floating === true,
                x: client.x,
                y: client.y,
                width: client.width,
                height: client.height,
                workspace: workspace === undefined || workspace === null ? null : workspace.id,
            };
        });
    }

    /** Every workspace as plain data, with the ids of the clients on it. */
    function workspaces() {
        const list = live().workspaces;
        return membersOf(list === undefined ? undefined : list.list).map((workspace) => ({
            id: workspace.id,
            name: workspace.name,
            index: workspace.index,
            active: workspace.active,
            layout: nameOf(workspace.layout),
            clients: membersOf(workspace.clients).map((client) => client.id),
        }));
    }

    /**
     * The bindings registered on every input context: the trigger, who owns it (the context, a layout or a device), its priority and its
     * `rank` among the bindings that match the same trigger. A shell with no seat has none.
     */
    function bindings() {
        const input = live().input;
        const contexts = input === undefined ? [] : membersOf(input.contexts);
        return contexts
            .flatMap((context) => Array.from(context.describeBindings()))
            .map((binding) => ({
                context: binding.context,
                id: binding.id,
                trigger: binding.trigger,
                kind: binding.kind,
                owner: binding.owner,
                layout: binding.layout === undefined ? null : binding.layout,
                device: binding.device === undefined ? null : binding.device,
                priority: binding.priority,
                enabled: binding.enabled,
                rank: binding.rank,
            }))
            .sort(
                (a, b) => a.context.localeCompare(b.context) || a.trigger.localeCompare(b.trigger) || a.rank - b.rank,
            );
    }

    return { clients, workspaces, bindings, render, plain };
})();
