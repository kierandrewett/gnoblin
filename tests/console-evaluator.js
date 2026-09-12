import {
    ConsoleEvaluator,
    isInspectable,
    preview,
} from "../src/gnome-shell-overlay/js/ui/components/gnoblinConsoleEvaluator.js";

function assert(condition, message) {
    if (!condition) throw new Error(message);
}

async function value(evaluator, source, expected) {
    const row = await evaluator.evaluate(source);
    assert(!row.error, `${source}: ${row.error?.message}\n${row.error?.stack}`);
    assert(row.value === expected, `${source}: expected ${expected}, got ${row.value}`);
    return row;
}

const logs = [];
let cleared = 0;
const evaluator = new ConsoleEvaluator(
    { answer: 41 },
    {
        onLog(level, values) {
            logs.push({ level, values });
        },
        onClear() {
            cleared++;
        },
    },
);

await value(evaluator, "answer + 1", 42);
await value(evaluator, "let count = answer; count", 41);
await value(evaluator, "count += 1; count", 42);
await value(evaluator, "const fixed = 9; fixed", 9);
const fixed = await evaluator.evaluate("fixed = 10");
assert(fixed.error instanceof TypeError, "const assignments reject");
await value(evaluator, "function readCount() { return count; } readCount()", 42);
await value(evaluator, "count = 7; readCount()", 7);
await value(evaluator, "class Box { constructor(value) { this.value = value; } }; new Box(3).value", 3);
await value(evaluator, "let {nested: renamed = 4, extra} = {nested: 8, extra: 2}; renamed + extra", 10);
await value(evaluator, "await Promise.resolve(count * 2)", 14);

let sideEffects = 0;
const failed = await evaluator.evaluate('sideEffects += 1; throw new Error("expected")');
assert(
    failed.error && sideEffects === 0,
    "a failed compilation/execution cannot mutate an external host binding implicitly",
);
// Scope values are deliberate console bindings.  A throw runs the program once.
await value(evaluator, "let hits = 0; hits", 0);
const thrown = await evaluator.evaluate('hits += 1; throw new Error("once")');
assert(thrown.error && (await evaluator.evaluate("hits")).value === 1, "throwing input executes once");

await value(evaluator, 'console.info("hello", count); $_', 1);
assert(logs.length === 1 && logs[0].level === "info" && logs[0].values[0] === "hello", "local console captures logs");
const row = await value(evaluator, "123", 123);
assert((await evaluator.evaluate(`r(${row.id})`)).value === 123, "result history reference");

const getterObject = { safe: 1 };
let getterCalls = 0;
Object.defineProperty(getterObject, "danger", {
    get() {
        getterCalls++;
        return 2;
    },
});
const getterEvaluator = new ConsoleEvaluator({ getterObject });
const completion = getterEvaluator.complete("getterObject.da");
assert(
    completion.items.some((item) => item.label === "danger") && getterCalls === 0,
    "completion does not invoke getters",
);
assert(
    getterEvaluator.complete("getterObject.").items.some((item) => item.label === "safe"),
    "completion after a dot lists properties",
);
const inspected = getterEvaluator.properties(getterObject);
assert(
    inspected.find((row) => row.name === "danger").accessor && getterCalls === 0,
    "inspection does not invoke getters",
);

const cycle = {};
cycle.self = cycle;
assert(preview(cycle).includes("self") && isInspectable(cycle) && !isInspectable(null), "bounded object preview");
for (let index = 0; index < 204; index++) await evaluator.evaluate(String(index));
assert(evaluator.result(1) === undefined && evaluator.result(204) !== undefined, "result history trims old rows");
evaluator.clear();
assert(
    cleared === 1 && evaluator.lastValue === undefined && (await evaluator.evaluate("count")).value === 7,
    "clear keeps lexical bindings",
);

const syntax = await evaluator.evaluate("let = ;");
assert(syntax.error instanceof SyntaxError, "syntax errors are returned");
print("PASS: compositor console evaluator");

const proto = {
    get inherited() {
        return this.own + 1;
    },
};
const object = Object.create(proto);
object.own = 4;
const symbol = Symbol("secret");
object[symbol] = 8;
Object.defineProperty(object, "hidden", { value: 12 });
Object.defineProperty(object, "throws", {
    get() {
        throw new Error("getter probe");
    },
});
const ownRows = evaluator.properties(object);
assert(
    ownRows.some((row) => row.name === "[Symbol(secret)]"),
    "symbol keys are inspectable",
);
assert(ownRows.find((row) => row.name === "hidden").flags.includes("non-enumerable"), "descriptor flags");
assert(!ownRows.some((row) => row.name === "inherited"), "prototype properties are separate");
const prototype = ownRows.find((row) => row.name === "[[Prototype]]");
assert(
    evaluator
        .properties(prototype.value, 0, 100, prototype.receiver)
        .find((row) => row.name === "inherited")
        .read() === 5,
    "inherited getters use the original receiver",
);
try {
    ownRows.find((row) => row.name === "throws").read();
    throw new Error("getter did not throw");
} catch (error) {
    assert(error.message === "getter probe", "getter errors are preserved");
}
const sparse = new Array(1000);
Object.defineProperty(sparse, "0", {
    get() {
        getterCalls++;
        return 1;
    },
});
assert(preview(sparse).includes("[Getter]") && getterCalls === 0, "array preview never reads accessors");
assert(
    evaluator.properties(Array.from({ length: 250 }, (_, i) => i)).some((row) => row.more === 100),
    "large arrays paginate",
);
assert(
    evaluator
        .properties(
            Array.from({ length: 250 }, (_, i) => i),
            200,
        )
        .some((row) => row.name === "249"),
    "last array page is reachable",
);
assert(
    evaluator.properties(new Map([["key", { a: 1 }]])).some((row) => row.value?.key === "key"),
    "map entries",
);
assert(
    evaluator.properties(new Set([4])).some((row) => row.value === 4),
    "set entries",
);
assert(preview({ answer: 42 }).includes("answer: 42"), "object preview includes values");
assert(preview(cycle).includes("[Circular]"), "circular object preview");
print("PASS: rich JavaScript inspection, getter safety, prototypes, symbols, collections and pagination");
