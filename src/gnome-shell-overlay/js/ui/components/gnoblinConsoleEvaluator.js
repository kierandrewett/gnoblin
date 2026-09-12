// A small evaluator for the Gnoblin compositor console.  It deliberately has
// no Shell dependency so that it can also be tested with a plain GJS process.

const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const MAX_RESULTS = 200;
const MAX_PREVIEW = 240;

function offsetFor(source, loc) {
    let offset = 0;
    let line = 1;
    while (line < loc.line) {
        const next = source.indexOf('\n', offset);
        if (next < 0)
            return source.length;
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
    if (!pattern)
        return names;
    if (pattern.type === 'Identifier')
        names.push(pattern.name);
    else if (pattern.type === 'RestElement')
        patternNames(pattern.argument, names);
    else if (pattern.type === 'AssignmentPattern' || pattern.type === 'AssignmentExpression')
        patternNames(pattern.left, names);
    else if (pattern.type === 'ArrayPattern')
        pattern.elements.forEach(part => patternNames(part, names));
    else if (pattern.type === 'ObjectPattern')
        pattern.properties.forEach(part => patternNames(part.value ?? part.argument, names));
    return names;
}

function replace(source, replacements) {
    for (const {start, end, text} of replacements.sort((a, b) => b.start - a.start))
        source = source.slice(0, start) + text + source.slice(end);
    return source;
}

function shiftLocations(node, lines) {
    if (!node || typeof node !== 'object')
        return;
    if (node.loc) {
        node.loc.start.line += lines;
        node.loc.end.line += lines;
    }
    for (const value of Object.values(node)) {
        if (Array.isArray(value))
            value.forEach(child => shiftLocations(child, lines));
        else if (value && typeof value === 'object')
            shiftLocations(value, lines);
    }
}

function parseProgram(source) {
    try {
        return Reflect.parse(source, {loc: true});
    } catch (error) {
        // Reflect.parse does not accept top-level await.  Parse the exact text
        // as an async function body so its AST can still guide rewriting.
        const wrapped = `async function __gnoblin_top_level__() {\n${source}\n}`;
        const tree = Reflect.parse(wrapped, {loc: true});
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
        if (descriptor)
            return descriptor;
        object = Object.getPrototypeOf(object);
    }
    return null;
}

export function preview(value, seen = new Set()) {
    try {
        if (value === null)
            return 'null';
        switch (typeof value) {
        case 'undefined': return 'undefined';
        case 'string': return JSON.stringify(value.length > MAX_PREVIEW ? `${value.slice(0, MAX_PREVIEW - 3)}...` : value);
        case 'number': case 'bigint': case 'boolean': return String(value);
        case 'symbol': return String(value);
        case 'function': return `[Function${value.name ? `: ${value.name}` : ''}]`;
        }
        if (seen.has(value))
            return '[Circular]';
        seen.add(value);
        if (Array.isArray(value))
            return `[${value.slice(0, 8).map(item => preview(item, seen)).join(', ')}${value.length > 8 ? ', ...' : ''}]`;
        const constructor = propertyDescriptor(value, 'constructor');
        const name = constructor && 'value' in constructor && typeof constructor.value === 'function'
            ? constructor.value.name || 'Object' : 'Object';
        const message = propertyDescriptor(value, 'message');
        if (name.endsWith('Error') && message && 'value' in message)
            return `${name}: ${String(message.value).slice(0, MAX_PREVIEW)}`;
        const keys = Object.getOwnPropertyNames(value);
        if (name === 'Object')
            return `{${keys.slice(0, 8).join(', ')}${keys.length > 8 ? ', ...' : ''}}`;
        return `[${name}${keys.length ? `: ${keys.slice(0, 8).join(', ')}${keys.length > 8 ? ', ...' : ''}` : ''}]`;
    } catch (error) {
        return `[Unpreviewable: ${error.message}]`;
    }
}

export function isInspectable(value) {
    return value !== null && (typeof value === 'object' || typeof value === 'function');
}

export class ConsoleEvaluator {
    constructor(bindings = {}, {onLog = () => {}, onInspect = () => {}, onClear = () => {}} = {}) {
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
        this._values.r = id => this.result(id);
        this._values.inspect = value => {
            this._onInspect(value);
            return value;
        };
        this._values.__gnoblin = this;
        this._scope = new Proxy(this._values, {
            has: () => true,
            get: (target, key) => own(target, key) ? target[key] : globalThis[key],
            set: (target, key, value) => {
                if (this._kinds.get(key) === 'const')
                    throw new TypeError(`Assignment to constant variable '${String(key)}'`);
                target[key] = value;
                return true;
            },
        });
    }

    _console() {
        const write = level => (...values) => this._onLog(level, values);
        return Object.freeze({log: write('log'), info: write('info'), warn: write('warn'), error: write('error'),
            clear: () => this.clear()});
    }

    _declare(kind, names, factory) {
        for (const name of names) {
            if ((kind === 'let' || kind === 'const') && own(this._values, name))
                throw new SyntaxError(`Identifier '${name}' has already been declared`);
            if (kind === 'var' && this._kinds.get(name) === 'const')
                throw new SyntaxError(`Identifier '${name}' has already been declared`);
        }
        const values = factory();
        names.forEach((name, index) => {
            this._values[name] = values[index];
            if (kind !== 'var')
                this._kinds.set(name, kind);
        });
        return values.at(-1);
    }

    _declareFunction(name, factory) {
        if (this._kinds.get(name) === 'const')
            throw new SyntaxError(`Identifier '${name}' has already been declared`);
        const value = factory();
        this._values[name] = value;
        return value;
    }

    _prepare(source) {
        const tree = parseProgram(source);
        const replacements = [];
        const body = tree.body;
        for (const statement of body) {
            if (statement.type === 'VariableDeclaration') {
                const [start, end] = span(source, statement);
                // Individual declarators now occupy this statement.  Keep the
                // separators out of the generated program.
                replacements.push({start, end, text: `${statement.declarations.map(declaration => {
                    const names = patternNames(declaration.id);
                    const pattern = source.slice(...span(source, declaration.id));
                    const initializer = declaration.init ? source.slice(...span(source, declaration.init)) : 'undefined';
                    return `__gnoblin._declare(${JSON.stringify(statement.kind)}, ${JSON.stringify(names)}, () => { let ${pattern} = (${initializer}); return [${names.join(', ')}]; })`;
                }).join(';')};`});
            } else if (statement.type === 'FunctionDeclaration' || statement.type === 'ClassDeclaration') {
                const [, end] = span(source, statement);
                // SpiderMonkey reports a declaration location from its name,
                // rather than from `function` or `class`.  The AST still gives
                // the declaration kind and end, so find its leading keyword.
                const keyword = statement.type === 'FunctionDeclaration' ? 'function' : 'class';
                const nameStart = offsetFor(source, statement.loc.start);
                const start = source.lastIndexOf(keyword, nameStart);
                const expression = source.slice(start, end);
                replacements.push({start, end, text: `__gnoblin._declareFunction(${JSON.stringify(statement.id.name)}, () => (${expression}));`});
            }
        }
        const last = body.at(-1);
        if (last?.type === 'ExpressionStatement') {
            const [start, end] = span(source, last.expression);
            replacements.push({start, end, text: `return (${source.slice(start, end)})`});
        }
        return replace(source, replacements);
    }

    async evaluate(source) {
        const started = Date.now();
        const row = {id: this._results.length ? this._results.at(-1).id + 1 : 1, source, value: undefined, error: null, durationMs: 0};
        try {
            const program = this._prepare(source);
            // AsyncFunction performs syntax compilation before this invocation.
            const execute = new AsyncFunction('scope', `with (scope) { return await (async function () { ${program}\n}).call(__gnoblin._scope); }`);
            row.value = await execute(this._scope);
            this._values.$_ = row.value;
            this._values.it = row.value;
        } catch (error) {
            row.error = error instanceof Error ? error : new Error(String(error));
        }
        row.durationMs = Date.now() - started;
        this._results.push(row);
        if (this._results.length > MAX_RESULTS)
            this._results.shift();
        return row;
    }

    result(id) {
        return this._results.find(row => row.id === Number(id))?.value;
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
        const match = before.match(/([A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)*)$/);
        if (!match)
            return {start: cursor, end: cursor, items: []};
        const path = match[1].split('.');
        const prefix = path.pop();
        let object = this._scope;
        for (const name of path) {
            const descriptor = propertyDescriptor(object, name);
            if (!descriptor || !('value' in descriptor))
                return {start: cursor - prefix.length, end: cursor, items: []};
            object = descriptor.value;
            if (!isInspectable(object))
                return {start: cursor - prefix.length, end: cursor, items: []};
        }
        const names = new Set();
        while (object && names.size < 500) {
            Object.getOwnPropertyNames(object).forEach(name => names.add(name));
            object = Object.getPrototypeOf(object);
        }
        const items = [...names].filter(name => name.startsWith(prefix)).sort().map(name => ({label: name, value: name}));
        return {start: cursor - prefix.length, end: cursor, items};
    }

    properties(value) {
        if (!isInspectable(value))
            return [];
        const rows = [];
        const names = new Set();
        let object = value;
        while (object && rows.length < 200) {
            for (const name of Object.getOwnPropertyNames(object)) {
                if (names.has(name))
                    continue;
                names.add(name);
                const descriptor = Object.getOwnPropertyDescriptor(object, name);
                const accessor = !('value' in descriptor);
                rows.push({name, value: accessor ? undefined : descriptor.value,
                    preview: accessor ? '[Accessor]' : preview(descriptor.value), accessor,
                    expandable: !accessor && isInspectable(descriptor.value)});
            }
            object = Object.getPrototypeOf(object);
        }
        return rows;
    }
}
