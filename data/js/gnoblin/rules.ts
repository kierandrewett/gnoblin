// Window rules: `defineConfig({ rules: [...] })`. A rule is data: a query over a window (appId, title, focused,
// onWorkspace) and the actions to take on a window that matches. Ported from data/lua/gnoblin/shell/rules.lua and
// `Window:apply_rules` of data/lua/gnoblin/shell/init.lua; docs/config/window_rule.md has the Lua rule reference and
// the table below the translation of it.
//
//   Lua rule (seat.data.window_rules)        JavaScript rule (defineConfig)
//   match = { app_id = "^foo$" }             appId: "foo"        a glob matches the whole text; * is any run, ? one char
//   match = { app_id = "fire" }              appId: "*fire*"     a Lua pattern matched anywhere; a glob needs the stars
//   match = { app_id = "^org%.x%.(%w+)$" }   appId: /^org\.x\.\w+$/   a RegExp is tested anywhere in the text, so anchor it
//   match = { title = "..." }                title: "..." or /.../
//   match = { focused = true }               focused: true
//   match = { workspace_id = "2" }           onWorkspace: "2"    (`workspace` is the action below)
//   workspace = { id = "3" }                 workspace: "3"
//   maximised / fullscreen / secure          the same names, true or false
//   opacity = 0.9                            style: { opacity: 0.9 }
//   (last matching rule wins per field)      the same: every matching rule applies in list order, the later wins per field
//   (stop at a rule)                         `first: true` on a rule: when it matches, no later rule is consulted
//   (re-evaluated on any rule input change)  re-applied when appId, title or focus changes (`reapply: false` opts a rule out)
//   xr = false, corners, blur, ...           not ported: the key is an unknown key and is refused by name
//
// New actions the Lua rules had no field for: `floating`, `size`, `position`, `skipFocus`.
// `role` has no source: the tree publishes no window role, so a rule that asks for one is refused by name.
//
// A rule that matches nothing changes nothing. A malformed rule throws at compile time (`defineConfig` runs it),
// naming the rule's index and the field, never at match time.

import { checkStyleInit } from "gnoblin/style";
import { rejectUnknown } from "gnoblin/workspace";

/** A plain glob (`*` any run of characters, `?` one) matching the whole text, or a RegExp tested anywhere in it. */
export type Pattern = string | RegExp;

export interface RuleStyle {
    cornerRadius?: number;
    /** From 0 to 1. Applied to the window. */
    opacity?: number;
    shadows?: readonly unknown[];
}

export interface RuleConfig {
    // The query: every key given must hold.
    appId?: Pattern;
    title?: Pattern;
    /** Refused: the tree publishes no window role. */
    role?: Pattern;
    /** Whether the window has the keyboard focus when the rule is read. */
    focused?: boolean;
    /** The id of the workspace the window is on when the rule is read. */
    onWorkspace?: string;
    // The actions.
    workspace?: string;
    floating?: boolean;
    size?: { width: number; height: number };
    position?: { x: number; y: number };
    style?: RuleStyle;
    fullscreen?: boolean;
    maximised?: boolean;
    secure?: boolean;
    /** The window does not take focus when it appears. */
    skipFocus?: boolean;
    // How the rule combines.
    /** When this rule matches, no later rule is consulted: the rules before it and this one still merge. */
    first?: boolean;
    /**
     * Apply the rule again when the window's appId, title or focus changes, and undo what it set when it stops matching.
     * The default is true, as every Lua rule was; `false` applies the rule once, when the window appears.
     */
    reapply?: boolean;
}

const QUERY_KEYS = ["appId", "title", "role", "focused", "onWorkspace"];
const ACTION_KEYS = [
    "workspace",
    "floating",
    "size",
    "position",
    "style",
    "fullscreen",
    "maximised",
    "secure",
    "skipFocus",
];
const OPTION_KEYS = ["first", "reapply"];
const RULE_KEYS = [...QUERY_KEYS, ...ACTION_KEYS, ...OPTION_KEYS];

/** A pattern, compiled: a glob becomes an anchored expression, text with no wildcard stays a plain comparison. */
interface CompiledPattern {
    readonly exact: string | null;
    readonly expression: RegExp | null;
}

/** What a rule sets on a window. */
export interface RuleEffects {
    workspace?: string;
    floating?: boolean;
    size?: { width: number; height: number };
    position?: { x: number; y: number };
    opacity?: number;
    cornerRadius?: number;
    shadows?: readonly unknown[];
    fullscreen?: boolean;
    maximised?: boolean;
    secure?: boolean;
    skipFocus?: boolean;
}

export interface CompiledRule {
    /** The rule's place in the list, for messages. */
    readonly index: number;
    readonly appId: CompiledPattern | null;
    readonly title: CompiledPattern | null;
    readonly focused: boolean | null;
    readonly onWorkspace: string | null;
    readonly effects: Readonly<RuleEffects>;
    readonly first: boolean;
    readonly reapply: boolean;
}

/** The window as a rule reads it: plain values, so matching stays pure. */
export interface RuleView {
    readonly appId: string;
    readonly title: string;
    readonly focused: boolean;
    /** The id of the window's workspace, or null. */
    readonly workspace: string | null;
}

function fail(where: string, message: string): never {
    throw new TypeError(`${where}: ${message}`);
}

/** A glob's expression: `*` is any run of characters (a newline too), `?` one, everything else is itself. */
function globToRegExp(glob: string): RegExp {
    let source = "";
    for (const ch of glob) {
        source += ch === "*" ? ".*" : ch === "?" ? "." : ch.replace(/[\\^$.+()[\]{}|/-]/g, "\\$&");
    }
    return new RegExp(`^${source}$`, "s");
}

export function compilePattern(value: unknown, where: string): CompiledPattern {
    if (typeof value === "string") {
        return /[*?]/.test(value)
            ? { exact: null, expression: globToRegExp(value) }
            : { exact: value, expression: null };
    }
    if (value instanceof RegExp) {
        // A `g` or `y` flag keeps a `lastIndex` between tests, which would make a match depend on the last window.
        return { exact: null, expression: new RegExp(value.source, value.flags.replace(/[gy]/g, "")) };
    }
    return fail(where, `must be a glob string or a RegExp, got ${describe(value)}`);
}

export function matchesPattern(pattern: CompiledPattern, text: string): boolean {
    return pattern.exact !== null ? text === pattern.exact : (pattern.expression as RegExp).test(text);
}

function describe(value: unknown): string {
    return typeof value === "string" ? JSON.stringify(value) : String(value);
}

function flag(rule: Record<string, unknown>, key: string, where: string): boolean | null {
    const value = rule[key];
    if (value === undefined) {
        return null;
    }
    if (typeof value !== "boolean") {
        fail(`${where}.${key}`, `must be true or false, got ${describe(value)}`);
    }
    return value;
}

function numbers(value: unknown, keys: readonly string[], where: string, positive: boolean): Record<string, number> {
    if (value === null || typeof value !== "object") {
        return fail(where, `must be an object with ${keys.join(" and ")}`);
    }
    rejectUnknown(value, keys, where);
    const out: Record<string, number> = {};
    for (const key of keys) {
        const n = (value as Record<string, unknown>)[key];
        if (typeof n !== "number" || !Number.isFinite(n) || (positive && n < 1)) {
            fail(`${where}.${key}`, `must be a ${positive ? "positive " : ""}finite number, got ${describe(n)}`);
        }
        out[key] = n;
    }
    return out;
}

function compileEffects(rule: RuleConfig, where: string, workspaces: ReadonlySet<string> | null): RuleEffects {
    const raw = rule as Record<string, unknown>;
    const effects: RuleEffects = {};
    if (rule.workspace !== undefined) {
        if (typeof rule.workspace !== "string" || rule.workspace === "") {
            fail(`${where}.workspace`, `must be a workspace id, got ${describe(rule.workspace)}`);
        }
        if (workspaces !== null && !workspaces.has(rule.workspace)) {
            throw new Error(
                `${where}.workspace: moves to workspace "${rule.workspace}", which the config does not declare`,
            );
        }
        effects.workspace = rule.workspace;
    }
    for (const key of ["floating", "fullscreen", "maximised", "secure", "skipFocus"] as const) {
        const value = flag(raw, key, where);
        if (value !== null) {
            effects[key] = value;
        }
    }
    if (rule.size !== undefined) {
        const size = numbers(rule.size, ["width", "height"], `${where}.size`, true);
        effects.size = { width: size.width, height: size.height };
    }
    if (rule.position !== undefined) {
        const position = numbers(rule.position, ["x", "y"], `${where}.position`, false);
        effects.position = { x: position.x, y: position.y };
    }
    if (rule.style !== undefined) {
        const style = rule.style as Record<string, unknown>;
        if (style === null || typeof style !== "object") {
            fail(`${where}.style`, "must be an object");
        }
        rejectUnknown(style, ["cornerRadius", "opacity", "shadows"], `${where}.style`);
        if (style.opacity !== undefined) {
            if (typeof style.opacity !== "number" || !(style.opacity >= 0 && style.opacity <= 1)) {
                fail(`${where}.style.opacity`, `must be a number from 0 to 1, got ${describe(style.opacity)}`);
            }
            effects.opacity = style.opacity;
        }
        if (style.cornerRadius !== undefined) {
            if (typeof style.cornerRadius !== "number" || !(style.cornerRadius >= 0)) {
                fail(
                    `${where}.style.cornerRadius`,
                    `must be a number of 0 or more, got ${describe(style.cornerRadius)}`,
                );
            }
            effects.cornerRadius = style.cornerRadius;
        }
        if (style.shadows !== undefined) {
            if (!Array.isArray(style.shadows)) {
                fail(`${where}.style.shadows`, "must be an array");
            }
            checkStyleInit({ shadows: style.shadows }, `${where}.style`);
            effects.shadows = style.shadows;
        }
    }
    return effects;
}

/**
 * Checks `rules` and compiles them. `workspaces` is the set of ids the config declares (a rule that names another
 * is refused), or null to skip that check, for a list a script builds after its workspaces exist.
 */
export function compileRules(
    rules: readonly RuleConfig[],
    workspaces: ReadonlySet<string> | null = null,
): CompiledRule[] {
    if (!Array.isArray(rules)) {
        throw new TypeError("rules must be an array");
    }
    return rules.map((rule, index) => {
        const where = `rules[${index}]`;
        if (rule === null || typeof rule !== "object") {
            fail(where, "must be an object");
        }
        rejectUnknown(rule, RULE_KEYS, where);
        const raw = rule as Record<string, unknown>;
        if (rule.role !== undefined) {
            fail(
                `${where}.role`,
                "the compositor publishes no window role, so a rule cannot match one; use appId or title",
            );
        }
        const appId = rule.appId === undefined ? null : compilePattern(rule.appId, `${where}.appId`);
        const title = rule.title === undefined ? null : compilePattern(rule.title, `${where}.title`);
        const focused = flag(raw, "focused", where);
        if (rule.onWorkspace !== undefined && (typeof rule.onWorkspace !== "string" || rule.onWorkspace === "")) {
            fail(`${where}.onWorkspace`, `must be a workspace id, got ${describe(rule.onWorkspace)}`);
        }
        if (appId === null && title === null && focused === null && rule.onWorkspace === undefined) {
            fail(where, "needs a query: appId, title, focused or onWorkspace");
        }
        const effects = compileEffects(rule, where, workspaces);
        if (Object.keys(effects).length === 0) {
            fail(where, `sets nothing; give it one of ${ACTION_KEYS.join(", ")}`);
        }
        return Object.freeze({
            index,
            appId,
            title,
            focused,
            onWorkspace: rule.onWorkspace ?? null,
            effects: Object.freeze(effects),
            first: flag(raw, "first", where) ?? false,
            reapply: flag(raw, "reapply", where) ?? true,
        });
    });
}

/** Whether one compiled rule's query holds for a window. */
export function ruleMatches(rule: CompiledRule, view: RuleView): boolean {
    return (
        (rule.appId === null || matchesPattern(rule.appId, view.appId)) &&
        (rule.title === null || matchesPattern(rule.title, view.title)) &&
        (rule.focused === null || rule.focused === view.focused) &&
        (rule.onWorkspace === null || rule.onWorkspace === view.workspace)
    );
}

/** The rules that apply to a window, in list order: every one that matches, up to and including a `first` rule. */
export function matchingRules(rules: readonly CompiledRule[], view: RuleView): CompiledRule[] {
    const out: CompiledRule[] = [];
    for (const rule of rules) {
        if (ruleMatches(rule, view)) {
            out.push(rule);
            if (rule.first) {
                break;
            }
        }
    }
    return out;
}

/** What a list of rules sets together: the later rule wins for each field, the others are kept. */
export function mergeEffects(rules: readonly CompiledRule[]): RuleEffects {
    const out: RuleEffects = {};
    for (const rule of rules) {
        Object.assign(out, rule.effects);
    }
    return out;
}

/** What a window takes from `rules`. Pure. */
export function effectsFor(rules: readonly CompiledRule[], view: RuleView): RuleEffects {
    return mergeEffects(matchingRules(rules, view));
}

/** What an undone rule leaves a window with: the value each reversible field has when no rule sets it. */
export const RULE_DEFAULTS = Object.freeze({ secure: false, fullscreen: false, maximised: false, opacity: 1 });

/** What the engine asks of the shell. `C` is the shell's client object. */
export interface RuleHost<C extends object> {
    view(client: C): RuleView;
    alive(client: C): boolean;
    /** Whether `client` is a window that appeared while the shell ran, not one an earlier generation already had. */
    isArrival(client: C): boolean;
    /** Moves the client to the workspace with this id. False when there is none. */
    moveToWorkspace(client: C, id: string): boolean;
    setFloating(client: C, on: boolean): void;
    /** The size a layout places the window at, before it has a position. */
    setSize(client: C, size: { width: number; height: number }): void;
    /** Places the window and holds it there against the layout's own placement. */
    setBounds(client: C, rect: { x: number; y: number; width: number; height: number }): void;
    /** The window's own rectangle now. */
    bounds(client: C): { x: number; y: number; width: number; height: number };
    /** Covers the output, or puts the window back where it was. */
    setFullscreen(client: C, on: boolean): void;
    /** Fills the work area, or puts the window back where it was. Refused while fullscreen. */
    setMaximised(client: C, on: boolean): void;
    setSecure(client: C, on: boolean): void;
    setOpacity(client: C, value: number): void;
    /**
     * Sets one style property of the client's own style, or takes it back to inheriting when `value` is `undefined`.
     * What a rule's `cornerRadius` and `shadows` become; `setOpacity` is the same for `opacity`.
     */
    setStyle(client: C, field: "cornerRadius" | "shadows", value: number | readonly unknown[] | undefined): void;
}

interface Applied {
    /** The indices of the rules that applied the last time the rules were read. */
    matched: Set<number>;
}

/**
 * Applies the rules to the windows of a collection: attach it to `gnoblin.clients`. A rule applies when its window
 * appears and again when appId, title or focus changes (unless the rule says `reapply: false`). A window an earlier generation already had is not
 * touched: a reload does not move the windows the user has placed.
 */
export class RuleEngine<C extends object> {
    readonly #rules: readonly CompiledRule[];
    readonly #host: RuleHost<C>;
    readonly #applied = new WeakMap<C, Applied>();
    readonly #skipFocus = new WeakSet<C>();
    readonly #reapplies: boolean;

    constructor(rules: readonly CompiledRule[], host: RuleHost<C>) {
        this.#rules = rules;
        this.#host = host;
        this.#reapplies = rules.some((rule) => rule.reapply);
    }

    /** Whether a rule asked that `client` not take focus. Read by the focus policy when the window is added. */
    skipsFocus(client: C): boolean {
        return this.#skipFocus.has(client);
    }

    onAdded(client: C): void {
        if (this.#rules.length === 0 || !this.#host.isArrival(client) || this.#applied.has(client)) {
            return;
        }
        const matched = matchingRules(this.#rules, this.#host.view(client));
        this.#applied.set(client, { matched: new Set(matched.map((rule) => rule.index)) });
        this.#apply(client, mergeEffects(matched));
    }

    onTitleChanged(client: C): void {
        this.#reapply(client);
    }

    onAppIdChanged(client: C): void {
        this.#reapply(client);
    }

    /** The rule query reads focus, so a change of focus is a change of the rules' input. */
    onFocusChanged(client: C): void {
        this.#reapply(client);
    }

    #reapply(client: C): void {
        const state = this.#applied.get(client);
        if (!this.#reapplies || state === undefined || !this.#host.alive(client)) {
            return;
        }
        const now = matchingRules(this.#rules, this.#host.view(client));
        const nowIndices = new Set(now.map((rule) => rule.index));
        const started = now.filter((rule) => rule.reapply && !state.matched.has(rule.index));
        const stopped = this.#rules.filter(
            (rule) => rule.reapply && state.matched.has(rule.index) && !nowIndices.has(rule.index),
        );
        for (const rule of this.#rules) {
            if (rule.reapply) {
                if (nowIndices.has(rule.index)) {
                    state.matched.add(rule.index);
                } else {
                    state.matched.delete(rule.index);
                }
            }
        }
        if (stopped.length > 0) {
            // A field a rule stops setting goes back to what a rule that still matches sets, else to its default.
            const kept = mergeEffects(now);
            const undone: RuleEffects = {};
            for (const field of Object.keys(RULE_DEFAULTS) as (keyof typeof RULE_DEFAULTS)[]) {
                if (stopped.some((rule) => rule.effects[field] !== undefined)) {
                    (undone as Record<string, unknown>)[field] = kept[field] ?? RULE_DEFAULTS[field];
                }
            }
            this.#apply(client, undone);
            for (const field of ["cornerRadius", "shadows"] as const) {
                if (stopped.some((rule) => rule.effects[field] !== undefined)) {
                    this.#host.setStyle(client, field, kept[field]);
                }
            }
        }
        if (started.length > 0) {
            // A rule that starts to match sets its fields, but a later rule that already matched keeps what it set.
            const merged = mergeEffects(now);
            const fresh: RuleEffects = {};
            for (const field of new Set(started.flatMap((rule) => Object.keys(rule.effects)))) {
                (fresh as Record<string, unknown>)[field] = (merged as Record<string, unknown>)[field];
            }
            this.#apply(client, fresh);
        }
    }

    /** Writes `effects` to the window, fullscreen before maximised: a fullscreen window does not maximise. */
    #apply(client: C, effects: RuleEffects): void {
        const host = this.#host;
        if (effects.workspace !== undefined && !host.moveToWorkspace(client, effects.workspace)) {
            throw new Error(
                `a window rule moves ${this.#name(client)} to workspace "${effects.workspace}", which does not exist`,
            );
        }
        if (effects.floating !== undefined) {
            host.setFloating(client, effects.floating);
        }
        if (effects.secure !== undefined) {
            host.setSecure(client, effects.secure);
        }
        if (effects.opacity !== undefined) {
            host.setOpacity(client, effects.opacity);
        }
        if (effects.cornerRadius !== undefined) {
            host.setStyle(client, "cornerRadius", effects.cornerRadius);
        }
        if (effects.shadows !== undefined) {
            host.setStyle(client, "shadows", effects.shadows);
        }
        this.#place(client, effects);
        if (effects.fullscreen !== undefined) {
            host.setFullscreen(client, effects.fullscreen);
        }
        if (effects.maximised !== undefined) {
            host.setMaximised(client, effects.maximised);
        }
        if (effects.skipFocus === true) {
            this.#skipFocus.add(client);
        }
    }

    /** A size alone is what the layout places the window at; a position holds the window where the rule put it. */
    #place(client: C, effects: RuleEffects): void {
        const { size, position } = effects;
        if (position === undefined) {
            if (size !== undefined) {
                this.#host.setSize(client, size);
            }
            return;
        }
        const own = this.#host.bounds(client);
        this.#host.setBounds(client, {
            x: position.x,
            y: position.y,
            width: size?.width ?? own.width,
            height: size?.height ?? own.height,
        });
    }

    #name(client: C): string {
        const appId = this.#host.view(client).appId;
        return appId === "" ? "a window" : appId;
    }
}
