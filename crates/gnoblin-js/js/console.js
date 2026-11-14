// The `console` global of every generation (`crates/gnoblin-js/src/script_console.rs` installs it).
//
// This file is evaluated once per generation as a function expression and called with `native`, the three host
// functions it needs: `admit()` says whether the generation may log another line (the rate limit), `emit(level, text,
// stack)` hands a finished line to the log, and `remap(stack)` rewrites the frames of a stack in `.ts` coordinates. It
// formats in JavaScript because the formatting is about JavaScript values (cycles, errors, getters), and it never throws
// at the caller: a console that threw would turn a debugging aid into a crash.
(function (native) {
    "use strict";

    // How deep and how wide a value is walked, so that logging a big graph costs a bounded amount.
    const MAX_DEPTH = 6;
    const MAX_ITEMS = 100;

    function number(value) {
        return Object.is(value, -0) ? "-0" : String(value);
    }

    function functionText(fn) {
        return fn.name ? "[Function: " + fn.name + "]" : "[Function (anonymous)]";
    }

    // An error as Node prints it: its name and message, then the stack with every frame in `.ts` coordinates. QuickJS's
    // `stack` holds the frames only, so the head is added unless the stack already starts with it.
    function errorText(error) {
        let name = "Error";
        let message = "";
        let stack = "";
        try {
            name = String(error.name || "Error");
            message = String(error.message === undefined ? "" : error.message);
            stack = typeof error.stack === "string" ? native.remap(error.stack).replace(/\n+$/, "") : "";
        } catch (_) {
            // An error whose properties throw is still an error: say what is known.
        }
        const head = message ? name + ": " + message : name;
        if (!stack) {
            return head;
        }
        return stack.startsWith(head) ? stack : head + "\n" + stack;
    }

    // A value as JSON-like text. Strings are quoted, a cycle is `[Circular]`, and what JSON cannot say (`undefined`,
    // a function, `NaN`, a symbol) is named, so a logged object shows what it holds.
    function toText(value, ancestors, depth) {
        switch (typeof value) {
            case "string":
                return JSON.stringify(value);
            case "number":
                return number(value);
            case "boolean":
            case "undefined":
            case "symbol":
                return String(value);
            case "function":
                return functionText(value);
            default:
                break;
        }
        if (value === null) {
            return "null";
        }
        if (ancestors.includes(value)) {
            return "[Circular]";
        }
        if (value instanceof Error) {
            return "[" + errorText(value).split("\n")[0] + "]";
        }
        if (value instanceof Date) {
            return Number.isNaN(value.getTime()) ? "Invalid Date" : JSON.stringify(value.toISOString());
        }
        if (value instanceof RegExp) {
            return String(value);
        }
        if (depth >= MAX_DEPTH) {
            return Array.isArray(value) ? "[Array]" : "[Object]";
        }
        ancestors.push(value);
        try {
            if (typeof value.toJSON === "function") {
                const data = value.toJSON();
                if (data !== value) {
                    return toText(data, ancestors, depth + 1);
                }
            }
            if (value instanceof Map) {
                const entries = [];
                for (const [key, item] of value) {
                    if (entries.length === MAX_ITEMS) {
                        entries.push("... " + (value.size - MAX_ITEMS) + " more entries");
                        break;
                    }
                    entries.push(toText(key, ancestors, depth + 1) + " => " + toText(item, ancestors, depth + 1));
                }
                return "Map(" + value.size + ") {" + entries.join(",") + "}";
            }
            if (value instanceof Set) {
                const items = [];
                for (const item of value) {
                    if (items.length === MAX_ITEMS) {
                        items.push("... " + (value.size - MAX_ITEMS) + " more items");
                        break;
                    }
                    items.push(toText(item, ancestors, depth + 1));
                }
                return "Set(" + value.size + ") {" + items.join(",") + "}";
            }
            const list = Array.isArray(value) ? value : ArrayBuffer.isView(value) ? Array.from(value) : null;
            if (list !== null) {
                const items = [];
                for (let index = 0; index < list.length; index += 1) {
                    if (index === MAX_ITEMS) {
                        items.push('"... ' + (list.length - MAX_ITEMS) + ' more items"');
                        break;
                    }
                    items.push(toText(list[index], ancestors, depth + 1));
                }
                return "[" + items.join(",") + "]";
            }
            const fields = [];
            const keys = Object.keys(value);
            for (const key of keys) {
                if (fields.length === MAX_ITEMS) {
                    fields.push('"...": "' + (keys.length - MAX_ITEMS) + ' more keys"');
                    break;
                }
                let item;
                try {
                    item = toText(value[key], ancestors, depth + 1);
                } catch (error) {
                    item = "[Getter threw: " + String(error && error.message) + "]";
                }
                fields.push(JSON.stringify(key) + ":" + item);
            }
            // An instance of a class names it, so `Client {...}` is not mistaken for a plain object.
            const proto = Object.getPrototypeOf(value);
            const owner = proto !== null && proto !== Object.prototype ? proto.constructor : null;
            const prefix = owner && typeof owner.name === "string" && owner.name ? owner.name + " " : "";
            return prefix + "{" + fields.join(",") + "}";
        } finally {
            ancestors.pop();
        }
    }

    // One argument that is not the format string: a string is itself, an error is its stack, the rest is JSON-like.
    function render(value) {
        if (typeof value === "string") {
            return value;
        }
        if (value instanceof Error) {
            return errorText(value);
        }
        return toText(value, [], 0);
    }

    // `%s %d %i %f %j %o %O %c %%`, as Node does.
    function specifier(code, arg) {
        switch (code) {
            case "s":
                if (typeof arg === "string") {
                    return arg;
                }
                if (typeof arg === "number") {
                    return number(arg);
                }
                return typeof arg === "object" && arg !== null ? render(arg) : String(arg);
            case "d":
                return typeof arg === "symbol" || (typeof arg === "object" && arg !== null)
                    ? "NaN"
                    : number(Number(arg));
            case "i":
                return typeof arg === "symbol" ? "NaN" : number(parseInt(arg, 10));
            case "f":
                return typeof arg === "symbol" ? "NaN" : number(parseFloat(arg));
            case "j":
                try {
                    const text = JSON.stringify(arg);
                    return text === undefined ? "undefined" : text;
                } catch (_) {
                    return "[Circular]";
                }
            case "c":
                return "";
            default:
                return typeof arg === "string" ? JSON.stringify(arg) : render(arg);
        }
    }

    function format(args) {
        if (args.length === 0) {
            return "";
        }
        let used = 1;
        let text;
        if (typeof args[0] === "string") {
            text = args[0];
            if (args.length > 1 && text.includes("%")) {
                text = text.replace(/%([sdifjoOc%])/g, (match, code) => {
                    if (code === "%") {
                        return "%";
                    }
                    if (used >= args.length) {
                        return match;
                    }
                    const arg = args[used];
                    used += 1;
                    return specifier(code, arg);
                });
            }
        } else {
            text = render(args[0]);
        }
        for (; used < args.length; used += 1) {
            text += " " + render(args[used]);
        }
        return text;
    }

    function emit(level, args) {
        if (!native.admit()) {
            return;
        }
        let text;
        try {
            text = format(args);
        } catch (error) {
            text = "console." + level + " could not format its arguments: " + String(error && error.message);
        }
        native.emit(level, text, new Error().stack);
    }

    const console = {
        log(...args) {
            emit("log", args);
        },
        info(...args) {
            emit("info", args);
        },
        warn(...args) {
            emit("warn", args);
        },
        error(...args) {
            emit("error", args);
        },
        debug(...args) {
            emit("debug", args);
        },
        // Logs at error level when `condition` is falsy, and says so; does not throw.
        assert(condition, ...args) {
            if (condition) {
                return;
            }
            if (typeof args[0] === "string") {
                args[0] = "Assertion failed: " + args[0];
            } else {
                args.unshift("Assertion failed");
            }
            emit("error", args);
        },
        // A dump of `data`, not a drawn table: the value as JSON-like text on one line.
        table(data) {
            emit("info", [typeof data === "object" && data !== null ? toText(data, [], 0) : data]);
        },
    };
    Object.defineProperty(globalThis, "console", {
        value: console,
        writable: true,
        configurable: true,
        enumerable: false,
    });
});
