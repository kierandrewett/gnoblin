// The simple tier: `export default defineConfig({...})`, plain data that desugars to the object API.
//
// `defineConfig` is an identity function with a typed parameter. It checks the shape, hands the config to the
// host (`globalThis.gnoblin.__register`, when the host provides one) and returns it. `desugarConfig` is the
// published table from design/js-research/spec-review.md section 4: it turns a config into the plan the host
// applies, and a plan is plain data, so a reload is a diff of two plans.
//
//   config key          plan                                                      host applies it as
//   gaps.inner/outer    workspaces[].gaps, passed to layouts as LayoutContext.gap  Workspace.gaps
//   style               style, stored as written (no native effect yet)            gnoblin.clients.style
//   outputs             outputs, stored as written (no native effect yet)          OutputProfile + Display
//   workspaces          workspaces[] with a Layout instance and the gaps           gnoblin.workspaces.add
//   keys                bindings[] of { trigger, action }, actions a closed union  input.primary.bind
//   rules               rules[], a data-only query and its actions                 clients.where(query)
//   on                  listeners[] of { event, handler }                          typed listener sugar
//   setup(gnoblin)      setup, run by the host after the declarative part          the full object API
//
// A key missing from the config adds nothing to the plan. The one exception is `workspaces`: a shell with
// nowhere to put a window is unusable, so no workspace means the baseline's single floating `desktop`.

import { checkTrigger } from "gnoblin/bindings";
import { Layout } from "gnoblin/layout";
import type { Gaps } from "gnoblin/layout";
import { FloatingLayout } from "gnoblin/layouts/floating";
import { GridLayout } from "gnoblin/layouts/grid";
import { MasterStackLayout } from "gnoblin/layouts/master-stack";
import { MonocleLayout } from "gnoblin/layouts/monocle";

/** What the host puts at `globalThis.gnoblin`. The library names no member of it beyond `__register`. */
export interface GnoblinHost {
    readonly [member: string]: unknown;
}

/** What a function key action receives. */
export interface KeyContext {
    readonly gnoblin: GnoblinHost;
}

export type KeyHandler = (context: KeyContext) => void;

/** Run a command directly, with no shell. */
export interface SpawnAction {
    readonly kind: "spawn";
    readonly argv: readonly string[];
}

/** Show a workspace on the display the input is on. */
export interface ShowAction {
    readonly kind: "show";
    readonly workspace: string;
}

/** The closed set of key actions. A closed set can be inspected and diffed; a function cannot. */
export type KeyAction = SpawnAction | ShowAction | "close" | KeyHandler;

/** A key action as the plan holds it: `"close"` and functions get a `kind` too. */
export type ResolvedAction =
    SpawnAction | ShowAction | { readonly kind: "close" } | { readonly kind: "call"; readonly handler: KeyHandler };

export interface ShadowConfig {
    x?: number;
    y?: number;
    blur?: number;
    spread?: number;
    color?: string;
}

/** Window style. Stored for the host; nothing applies it natively yet. */
export interface StyleConfig {
    cornerRadius?: number;
    opacity?: number;
    shadows?: readonly ShadowConfig[];
}

/** Hardware profile of one output. Stored for the host; nothing applies it natively yet. */
export interface OutputConfig {
    mode?: string;
    scale?: number;
}

export type StockLayoutName = "floating" | "master-stack" | "grid" | "monocle";

export interface WorkspaceConfig {
    id: string;
    /** A stock layout by name, or an instance of any Layout. Default `floating`. */
    layout?: StockLayoutName | Layout;
}

/** A data-only rule: the query keys `appId` and `title` (exact match), the action keys `workspace` and `style`. */
export interface RuleConfig {
    appId?: string;
    title?: string;
    workspace?: string;
    style?: StyleConfig;
}

export interface ConfigListeners {
    clientAdded?(client: unknown): void;
    clientRemoved?(client: unknown): void;
    titleChanged?(client: unknown, title: string): void;
}

export interface GnoblinConfig {
    gaps?: { inner?: number; outer?: number };
    style?: StyleConfig;
    outputs?: Readonly<Record<string, OutputConfig>>;
    workspaces?: readonly WorkspaceConfig[];
    keys?: Readonly<Record<string, KeyAction>>;
    rules?: readonly RuleConfig[];
    on?: ConfigListeners;
    /** Runs after the declarative part and may use any class of the object API. */
    setup?(gnoblin: GnoblinHost): void;
}

export interface PlannedWorkspace {
    readonly id: string;
    readonly layout: Layout;
    readonly gaps: Gaps;
}

export interface PlannedKey {
    readonly trigger: string;
    readonly action: ResolvedAction;
}

export interface PlannedListener {
    readonly event: keyof ConfigListeners;
    readonly handler: (...args: never[]) => void;
}

export interface ConfigPlan {
    readonly gaps: Gaps;
    readonly style: StyleConfig;
    readonly outputs: Readonly<Record<string, OutputConfig>>;
    readonly workspaces: readonly PlannedWorkspace[];
    readonly bindings: readonly PlannedKey[];
    readonly rules: readonly RuleConfig[];
    readonly listeners: readonly PlannedListener[];
    readonly setup: ((gnoblin: GnoblinHost) => void) | undefined;
}

/** `spawn("ghostty")` or `spawn("wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle")`; an argv array works too. */
export function spawn(command: string | readonly string[], ...args: string[]): SpawnAction {
    const argv = typeof command === "string" ? [command, ...args] : [...command, ...args];
    if (argv.length === 0 || argv.some((part) => typeof part !== "string") || argv[0] === "") {
        throw new TypeError("spawn needs a command name and string arguments");
    }
    return Object.freeze({ kind: "spawn", argv: Object.freeze(argv) });
}

function showWorkspace(workspace: string | { readonly id: string }): ShowAction {
    const id = typeof workspace === "string" ? workspace : workspace?.id;
    if (typeof id !== "string" || id === "") {
        throw new TypeError("show needs a workspace id or a workspace config");
    }
    return Object.freeze({ kind: "show", workspace: id });
}

/** The most `show.numbered` binds: one digit key each, `1` to `9`. */
const MAX_NUMBERED = 9;

/**
 * `show.numbered("Super+{n}", ["desktop", "dev"])` binds Super+1 to `show("desktop")` and Super+2 to
 * `show("dev")`: `{n}` is the 1-based position of the workspace. Spread the result into `keys`.
 */
function numbered(template: string, workspaces: readonly (string | { readonly id: string })[]) {
    if (typeof template !== "string" || !template.includes("{n}")) {
        throw new TypeError(`show.numbered needs a template with {n}, such as "Super+{n}"`);
    }
    if (workspaces.length > MAX_NUMBERED) {
        throw new RangeError(`show.numbered binds at most ${MAX_NUMBERED} workspaces, got ${workspaces.length}`);
    }
    const keys: Record<string, ShowAction> = {};
    workspaces.forEach((workspace, i) => {
        keys[template.replaceAll("{n}", String(i + 1))] = showWorkspace(workspace);
    });
    return keys;
}

export const show = Object.assign(showWorkspace, { numbered });

const TOP_LEVEL_KEYS = ["gaps", "style", "outputs", "workspaces", "keys", "rules", "on", "setup"];
const LISTENER_EVENTS = ["clientAdded", "clientRemoved", "titleChanged"];
const RULE_KEYS = ["appId", "title", "workspace", "style"];

const STOCK_LAYOUTS: Readonly<Record<StockLayoutName, () => Layout>> = {
    floating: () => new FloatingLayout(),
    "master-stack": () => new MasterStackLayout(),
    grid: () => new GridLayout(),
    monocle: () => new MonocleLayout(),
};

/** The config the shell runs when the user has none: one floating `desktop` workspace and the stock keys. */
export const defaultConfig: Readonly<GnoblinConfig> = deepFreeze({
    workspaces: [{ id: "desktop", layout: "floating" }],
    keys: {
        ...show.numbered("Super+{n}", ["desktop"]),
        XF86AudioRaiseVolume: spawn("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"),
        XF86AudioLowerVolume: spawn("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"),
        XF86AudioMute: spawn("wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"),
        XF86AudioMicMute: spawn("wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"),
        XF86AudioPlay: spawn("playerctl", "play-pause"),
        XF86AudioNext: spawn("playerctl", "next"),
        XF86AudioPrev: spawn("playerctl", "previous"),
    },
} satisfies GnoblinConfig);

/**
 * Checks `config`, registers it with the host and returns it unchanged. Throws a TypeError or RangeError
 * that names the offending key, so a mistake fails the stage and keeps the running generation.
 */
export function defineConfig(config: GnoblinConfig): GnoblinConfig {
    desugarConfig(config);
    const host = (globalThis as { gnoblin?: { __register?: (config: GnoblinConfig) => void } }).gnoblin;
    host?.__register?.(config);
    return config;
}

/** The plan for `config`, see the table at the top of this file. Pure: the same config gives an equal plan. */
export function desugarConfig(config: GnoblinConfig): ConfigPlan {
    if (config === null || typeof config !== "object") {
        throw new TypeError("a config must be an object");
    }
    rejectUnknown(config, TOP_LEVEL_KEYS, "config");
    const gaps = checkGaps(config.gaps);
    const workspaces = planWorkspaces(config.workspaces, gaps);
    const ids = new Set(workspaces.map((workspace) => workspace.id));
    return {
        gaps,
        style: config.style ?? {},
        outputs: config.outputs ?? {},
        workspaces,
        bindings: planKeys(config.keys ?? {}, ids),
        rules: planRules(config.rules ?? [], ids),
        listeners: planListeners(config.on ?? {}),
        setup: checkSetup(config.setup),
    };
}

function rejectUnknown(object: object, known: readonly string[], what: string): void {
    for (const key of Object.keys(object)) {
        if (!known.includes(key)) {
            throw new TypeError(`${what} has no "${key}"; expected one of ${known.join(", ")}`);
        }
    }
}

function checkGaps(gaps: GnoblinConfig["gaps"]): Gaps {
    if (gaps === undefined) {
        return { inner: 0, outer: 0 };
    }
    rejectUnknown(gaps, ["inner", "outer"], "gaps");
    const read = (name: "inner" | "outer"): number => {
        const value = gaps[name] ?? 0;
        if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
            throw new RangeError(`gaps.${name} must be a finite number of at least 0, got ${String(value)}`);
        }
        return value;
    };
    return { inner: read("inner"), outer: read("outer") };
}

function planWorkspaces(declared: GnoblinConfig["workspaces"], gaps: Gaps): PlannedWorkspace[] {
    const list = declared === undefined || declared.length === 0 ? (defaultConfig.workspaces ?? []) : declared;
    const seen = new Set<string>();
    return list.map((workspace, i) => {
        const where = `workspaces[${i}]`;
        rejectUnknown(workspace, ["id", "layout"], where);
        if (typeof workspace.id !== "string" || workspace.id === "") {
            throw new TypeError(`${where}.id must be a non-empty string`);
        }
        if (seen.has(workspace.id)) {
            throw new Error(`${where}: a workspace with id "${workspace.id}" is already declared`);
        }
        seen.add(workspace.id);
        return { id: workspace.id, layout: resolveLayout(workspace.layout, where), gaps };
    });
}

function resolveLayout(layout: WorkspaceConfig["layout"], where: string): Layout {
    if (layout === undefined) {
        return STOCK_LAYOUTS.floating();
    }
    if (layout instanceof Layout) {
        return layout;
    }
    if (typeof layout === "string" && Object.hasOwn(STOCK_LAYOUTS, layout)) {
        return STOCK_LAYOUTS[layout]();
    }
    throw new TypeError(
        `${where}.layout must be a Layout or one of ${Object.keys(STOCK_LAYOUTS).join(", ")}, got ${String(layout)}`,
    );
}

function planKeys(keys: Readonly<Record<string, KeyAction>>, workspaces: ReadonlySet<string>): PlannedKey[] {
    return Object.entries(keys).map(([trigger, action]) => {
        checkTrigger(trigger, "a key");
        return { trigger, action: resolveAction(action, `keys["${trigger}"]`, workspaces) };
    });
}

function resolveAction(action: KeyAction, where: string, workspaces: ReadonlySet<string>): ResolvedAction {
    if (action === "close") {
        return { kind: "close" };
    }
    if (typeof action === "function") {
        return { kind: "call", handler: action };
    }
    if (action !== null && typeof action === "object") {
        if (action.kind === "spawn" && Array.isArray(action.argv) && action.argv.length > 0) {
            return action;
        }
        if (action.kind === "show" && typeof action.workspace === "string") {
            if (!workspaces.has(action.workspace)) {
                throw new Error(`${where} shows workspace "${action.workspace}", which the config does not declare`);
            }
            return action;
        }
    }
    throw new TypeError(`${where} must be spawn(...), show(...), "close" or a function`);
}

function planRules(rules: readonly RuleConfig[], workspaces: ReadonlySet<string>): RuleConfig[] {
    return rules.map((rule, i) => {
        const where = `rules[${i}]`;
        rejectUnknown(rule, RULE_KEYS, where);
        if (rule.appId === undefined && rule.title === undefined) {
            throw new TypeError(`${where} needs a query: appId or title`);
        }
        for (const key of ["appId", "title", "workspace"] as const) {
            if (rule[key] !== undefined && typeof rule[key] !== "string") {
                throw new TypeError(`${where}.${key} must be a string`);
            }
        }
        if (rule.workspace !== undefined && !workspaces.has(rule.workspace)) {
            throw new Error(`${where} moves to workspace "${rule.workspace}", which the config does not declare`);
        }
        return rule;
    });
}

function planListeners(on: ConfigListeners): PlannedListener[] {
    rejectUnknown(on, LISTENER_EVENTS, "on");
    return Object.entries(on).map(([event, handler]) => {
        if (typeof handler !== "function") {
            throw new TypeError(`on.${event} must be a function`);
        }
        return { event: event as keyof ConfigListeners, handler: handler as PlannedListener["handler"] };
    });
}

function checkSetup(setup: GnoblinConfig["setup"]): GnoblinConfig["setup"] {
    if (setup !== undefined && typeof setup !== "function") {
        throw new TypeError("setup must be a function");
    }
    return setup;
}

function deepFreeze<T extends object>(value: T): T {
    for (const member of Object.values(value)) {
        if (member !== null && typeof member === "object" && !Object.isFrozen(member)) {
            deepFreeze(member);
        }
    }
    return Object.freeze(value);
}
