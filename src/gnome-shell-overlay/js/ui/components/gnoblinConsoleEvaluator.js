// A small evaluator for the Gnoblin compositor console.  It deliberately has
// no Shell dependency so that it can also be tested with a plain GJS process.

const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const MAX_RESULTS = 200;
const MAX_PREVIEW = 240;

function offsetFor(source, loc) {
    let offset = 0;
    let line = 1;
    while (line < loc.line) {
        const next = source.indexOf("\n", offset);
        if (next < 0) return source.length;
        offset = next + 1;
        line++;
    }
    // Reflect.parse in this GJS version reports one-based columns.
    return offset + loc.column - 1;
}

function span(source, node) {
    return [offsetFor(source, node.loc.start), offsetFor(source, node.loc.end)];
}

function patternNames(pattern, names = []) {
    if (!pattern) return names;
    if (pattern.type === "Identifier") names.push(pattern.name);
    else if (pattern.type === "RestElement") patternNames(pattern.argument, names);
    else if (pattern.type === "AssignmentPattern" || pattern.type === "AssignmentExpression")
        patternNames(pattern.left, names);
    else if (pattern.type === "ArrayPattern") pattern.elements.forEach((part) => patternNames(part, names));
    else if (pattern.type === "ObjectPattern")
        pattern.properties.forEach((part) => patternNames(part.value ?? part.argument, names));
    return names;
}

function replace(source, replacements) {
    for (const { start, end, text } of replacements.sort((a, b) => b.start - a.start))
        source = source.slice(0, start) + text + source.slice(end);
    return source;
}

function shiftLocations(node, lines) {
    if (!node || typeof node !== "object") return;
    if (node.loc) {
        node.loc.start.line += lines;
        node.loc.end.line += lines;
    }
    for (const value of Object.values(node)) {
        if (Array.isArray(value)) value.forEach((child) => shiftLocations(child, lines));
        else if (value && typeof value === "object") shiftLocations(value, lines);
    }
}

function parseProgram(source) {
    try {
        return Reflect.parse(source, { loc: true });
    } catch (error) {
        // Reflect.parse does not accept top-level await.  Parse the exact text
        // as an async function body so its AST can still guide rewriting.
        const wrapped = `async function __gnoblin_top_level__() {\n${source}\n}`;
        const tree = Reflect.parse(wrapped, { loc: true });
        const body = tree.body[0].body;
        shiftLocations(body, -1);
        return body;
    }
}

function own(object, key) {
    return Object.prototype.hasOwnProperty.call(object, key);
}

function propertyDescriptor(object, key) {
    while (object) {
        const descriptor = Object.getOwnPropertyDescriptor(object, key);
        if (descriptor) return descriptor;
        object = Object.getPrototypeOf(object);
    }
    return null;
}

export function preview(value, seen = new Set()) {
    try {
        if (value instanceof LuaValue) return value.preview;
        if (value === null) return "null";
        switch (typeof value) {
            case "undefined":
                return "undefined";
            case "string":
                return JSON.stringify(value.length > MAX_PREVIEW ? `${value.slice(0, MAX_PREVIEW - 3)}...` : value);
            case "number":
            case "bigint":
            case "boolean":
                return String(value);
            case "symbol":
                return String(value);
            case "function": {
                const name = propertyDescriptor(value, "name")?.value;
                return `[Function${typeof name === "string" && name ? `: ${name}` : ""}]`;
            }
        }
        if (seen.has(value)) return "[Circular]";
        seen.add(value);
        const summarize = (item) => (seen.size > 3 && isInspectable(item) ? "…" : preview(item, new Set(seen)));
        if (Array.isArray(value)) {
            const count = Object.getOwnPropertyDescriptor(value, "length").value;
            const parts = [];
            for (let i = 0; i < Math.min(count, 6); i++) {
                const d = Object.getOwnPropertyDescriptor(value, String(i));
                parts.push(!d ? "<empty>" : "value" in d ? summarize(d.value) : "[Getter]");
            }
            return `Array(${count}) [${parts.join(", ")}${count > 6 ? ", …" : ""}]`;
        }
        const constructor = propertyDescriptor(value, "constructor");
        const name =
            constructor && "value" in constructor && typeof constructor.value === "function"
                ? constructor.value.name || "Object"
                : "Object";
        if (value instanceof Map) {
            const size = Object.getOwnPropertyDescriptor(Map.prototype, "size").get.call(value);
            return `Map(${size})`;
        }
        if (value instanceof Set) {
            const size = Object.getOwnPropertyDescriptor(Set.prototype, "size").get.call(value);
            return `Set(${size})`;
        }
        const message = propertyDescriptor(value, "message");
        if (name.endsWith("Error") && message && "value" in message)
            return `${name}: ${String(message.value).slice(0, MAX_PREVIEW)}`;
        const keys = Reflect.ownKeys(value);
        const parts = keys.slice(0, 5).map((key) => {
            const d = Object.getOwnPropertyDescriptor(value, key);
            return `${String(key)}: ${d && "value" in d ? summarize(d.value) : "[Getter]"}`;
        });
        return `${name} {${parts.join(", ")}${keys.length > 5 ? ", …" : ""}}`.slice(0, MAX_PREVIEW);
    } catch (error) {
        return `[Unpreviewable: ${error.message}]`;
    }
}

export function isInspectable(value) {
    return value instanceof LuaValue
        ? value.handle > 0
        : value !== null && (typeof value === "object" || typeof value === "function");
}

export class ConsoleEvaluator {
    constructor(bindings = {}, { onLog = () => {}, onInspect = () => {}, onClear = () => {} } = {}) {
        this._values = Object.create(null);
        this._kinds = new Map();
        this._results = [];
        this._onLog = onLog;
        this._onInspect = onInspect;
        this._onClear = onClear;
        Object.assign(this._values, bindings);
        this._values.console = this._console();
        this._values.$_ = undefined;
        this._values.it = undefined;
        this._values.r = (id) => this.result(id);
        this._values.inspect = (value) => {
            this._onInspect(value);
            return value;
        };
        this._values.__gnoblin = this;
        this._scope = new Proxy(this._values, {
            has: () => true,
            get: (target, key) => (own(target, key) ? target[key] : globalThis[key]),
            set: (target, key, value) => {
                if (this._kinds.get(key) === "const")
                    throw new TypeError(`Assignment to constant variable '${String(key)}'`);
                target[key] = value;
                return true;
            },
        });
    }

    _console() {
        const write =
            (level) =>
            (...values) =>
                this._onLog(level, values);
        return Object.freeze({
            log: write("log"),
            info: write("info"),
            warn: write("warn"),
            error: write("error"),
            clear: () => this.clear(),
        });
    }

    _declare(kind, names, factory) {
        for (const name of names) {
            if ((kind === "let" || kind === "const") && own(this._values, name))
                throw new SyntaxError(`Identifier '${name}' has already been declared`);
            if (kind === "var" && this._kinds.get(name) === "const")
                throw new SyntaxError(`Identifier '${name}' has already been declared`);
        }
        const values = factory();
        names.forEach((name, index) => {
            this._values[name] = values[index];
            if (kind !== "var") this._kinds.set(name, kind);
        });
        return values.at(-1);
    }

    _declareFunction(name, factory) {
        if (this._kinds.get(name) === "const") throw new SyntaxError(`Identifier '${name}' has already been declared`);
        const value = factory();
        this._values[name] = value;
        return value;
    }

    _prepare(source) {
        const tree = parseProgram(source);
        const replacements = [];
        const body = tree.body;
        for (const statement of body) {
            if (statement.type === "VariableDeclaration") {
                const [start, end] = span(source, statement);
                // Individual declarators now occupy this statement.  Keep the
                // separators out of the generated program.
                replacements.push({
                    start,
                    end,
                    text: `${statement.declarations
                        .map((declaration) => {
                            const names = patternNames(declaration.id);
                            const pattern = source.slice(...span(source, declaration.id));
                            const initializer = declaration.init
                                ? source.slice(...span(source, declaration.init))
                                : "undefined";
                            return `__gnoblin._declare(${JSON.stringify(statement.kind)}, ${JSON.stringify(names)}, () => { let ${pattern} = (${initializer}); return [${names.join(", ")}]; })`;
                        })
                        .join(";")};`,
                });
            } else if (statement.type === "FunctionDeclaration" || statement.type === "ClassDeclaration") {
                const [, end] = span(source, statement);
                // SpiderMonkey reports a declaration location from its name,
                // rather than from `function` or `class`.  The AST still gives
                // the declaration kind and end, so find its leading keyword.
                const keyword = statement.type === "FunctionDeclaration" ? "function" : "class";
                const nameStart = offsetFor(source, statement.loc.start);
                const start = source.lastIndexOf(keyword, nameStart);
                const expression = source.slice(start, end);
                replacements.push({
                    start,
                    end,
                    text: `__gnoblin._declareFunction(${JSON.stringify(statement.id.name)}, () => (${expression}));`,
                });
            }
        }
        const last = body.at(-1);
        if (last?.type === "ExpressionStatement") {
            const [start, end] = span(source, last.expression);
            replacements.push({ start, end, text: `return (${source.slice(start, end)})` });
        }
        return replace(source, replacements);
    }

    async evaluate(source) {
        const started = Date.now();
        const row = {
            id: this._results.length ? this._results.at(-1).id + 1 : 1,
            source,
            value: undefined,
            error: null,
            durationMs: 0,
        };
        try {
            const program = this._prepare(source);
            // AsyncFunction performs syntax compilation before this invocation.
            const execute = new AsyncFunction(
                "scope",
                `with (scope) { return await (async function () { ${program}\n}).call(__gnoblin._scope); }`,
            );
            row.value = await execute(this._scope);
            this._values.$_ = row.value;
            this._values.it = row.value;
        } catch (error) {
            row.error = error instanceof Error ? error : new Error(String(error));
        }
        row.durationMs = Date.now() - started;
        this._results.push(row);
        if (this._results.length > MAX_RESULTS) this._results.shift();
        return row;
    }

    result(id) {
        return this._results.find((row) => row.id === Number(id))?.value;
    }

    get lastValue() {
        return this._values.$_;
    }

    clear() {
        this._results = [];
        this._values.$_ = undefined;
        this._values.it = undefined;
        this._onClear();
    }

    complete(text, cursor = text.length) {
        const before = text.slice(0, cursor);
        const match = before.match(/([A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)*\.?)$/);
        if (!match) return { start: cursor, end: cursor, items: [] };
        const path = match[1].split(".");
        const prefix = path.pop();
        let object = this._scope;
        for (const name of path) {
            const descriptor = propertyDescriptor(object, name);
            if (!descriptor || !("value" in descriptor))
                return { start: cursor - prefix.length, end: cursor, items: [] };
            object = descriptor.value;
            if (!isInspectable(object)) return { start: cursor - prefix.length, end: cursor, items: [] };
        }
        const names = new Set();
        while (object && names.size < 500) {
            Object.getOwnPropertyNames(object).forEach((name) => names.add(name));
            object = Object.getPrototypeOf(object);
        }
        const items = [...names]
            .filter((name) => name.startsWith(prefix))
            .sort()
            .map((name) => ({ label: name, value: name }));
        return { start: cursor - prefix.length, end: cursor, items };
    }

    properties(value, offset = 0, limit = 100, receiver = value) {
        if (!isInspectable(value)) return [];
        try {
            const rows = [];
            if (value instanceof Map || value instanceof Set) {
                const map = value instanceof Map;
                const entries = map ? Map.prototype.entries.call(value) : Set.prototype.values.call(value);
                const size = Object.getOwnPropertyDescriptor(map ? Map.prototype : Set.prototype, "size").get.call(
                    value,
                );
                let i = 0;
                for (const entry of entries) {
                    if (i >= offset + limit) break;
                    if (i >= offset)
                        rows.push({ name: `[${i}]`, value: map ? { key: entry[0], value: entry[1] } : entry });
                    i++;
                }
                if (offset + limit < size)
                    rows.push({
                        more: offset + limit,
                        name: `Show next ${Math.min(limit, size - offset - limit)} entries`,
                    });
                if (!offset) {
                    rows.unshift({ name: "size", value: size });
                    rows.push({ name: "[[Prototype]]", value: Object.getPrototypeOf(value), receiver });
                }
                return rows;
            }
            const keys = Reflect.ownKeys(value);
            for (const key of keys.slice(offset, offset + limit)) {
                const descriptor = Object.getOwnPropertyDescriptor(value, key);
                if (!descriptor) continue;
                const accessor = !("value" in descriptor);
                rows.push({
                    name: typeof key === "symbol" ? `[${String(key)}]` : key,
                    value: descriptor.value,
                    accessor,
                    enumerable: descriptor.enumerable,
                    preview: descriptor.get ? "[Getter]" : "[Setter]",
                    read: descriptor.get ? () => descriptor.get.call(receiver) : null,
                    flags: [
                        descriptor.enumerable ? "" : "non-enumerable",
                        descriptor.writable === false ? "read-only" : "",
                        descriptor.configurable ? "" : "non-configurable",
                    ]
                        .filter(Boolean)
                        .join(", "),
                });
            }
            if (offset + limit < keys.length)
                rows.push({
                    more: offset + limit,
                    name: `Show next ${Math.min(limit, keys.length - offset - limit)} properties`,
                });
            if (!offset) {
                const prototype = Object.getPrototypeOf(value);
                if (prototype !== null)
                    rows.push({ name: "[[Prototype]]", value: prototype, receiver, enumerable: false });
            }
            return rows;
        } catch (error) {
            return [{ name: "[[Inspection error]]", value: error.message }];
        }
    }
}

export class LuaValue {
    constructor(detail, owner) {
        Object.assign(this, detail);
        this.owner = owner;
        this.generation = owner._generation;
    }
}

// The native Lua state uses the same restricted libraries and gnoblin module
// as config evaluation. It is independent of both the config loader and GJS.
export class LuaConsoleEvaluator {
    constructor(invoke, onLog) {
        this._invoke = invoke;
        this._objects = new Map();
        this._generation = 0;
        this._onLog = onLog;
        this._results = [];
        this._nextId = 1;
    }

    async evaluate(source) {
        const started = Date.now();
        const reply = this._invoke("eval", source);
        for (const line of reply.lines ?? []) this._onLog(line);
        const error = reply.error ? new Error(reply.error.split("\n")[0]) : null;
        if (error) {
            error.name = "LuaError";
            error.stack = reply.error;
        }
        const row = {
            id: this._nextId++,
            source,
            value: (reply.values ?? []).join("\t"),
            error,
            durationMs: Date.now() - started,
            lua: true,
            items: reply.details
                ? reply.details.map((detail) => this._wrap(detail))
                : [
                      reply.inspectionError
                          ? `Inspection unavailable: ${reply.inspectionError}`
                          : (reply.values ?? []).join("\t"),
                  ],
        };
        this._results.push(row);
        if (this._results.length > 200) this._results.shift();
        return row;
    }

    complete(text, cursor = text.length) {
        const match = text.slice(0, cursor).match(/([A-Za-z_][\w]*(?:\.[A-Za-z_][\w]*)*\.?)$/);
        if (!match) return { start: cursor, end: cursor, items: [] };
        const prefix = match[0].split(".").at(-1);
        const reply = this._invoke("complete", match[0]);
        return {
            start: cursor - prefix.length,
            end: cursor,
            items: (reply.lines ?? []).sort().map((name) => ({ label: name, value: name })),
        };
    }

    get lastValue() {
        return this._results.at(-1)?.value;
    }
    result(id) {
        return this._results.find((row) => row.id === id)?.value;
    }
    _wrap(detail) {
        if (detail.handle && this._objects.has(detail.handle)) return this._objects.get(detail.handle);
        const value = new LuaValue(detail, this);
        if (detail.handle) this._objects.set(detail.handle, value);
        return value;
    }

    properties(value, offset = 0) {
        if (value.generation !== this._generation) return [{ name: "[[Expired]]", value: "Lua context was reset" }];
        const reply = this._invoke("inspect", `${value.handle}:${offset}`);
        if (reply.error) return [{ name: "[[Inspection error]]", value: reply.error }];
        return (reply.details ?? []).map((row) =>
            row.more !== undefined
                ? row
                : {
                      ...row,
                      value: this._wrap(row.value),
                      key: row.key?.handle ? this._wrap(row.key) : undefined,
                  },
        );
    }
    clear() {
        this._results = [];
    }
    reset() {
        this._invoke("reset", "");
        this._generation++;
        this._objects.clear();
        this.clear();
    }
}
