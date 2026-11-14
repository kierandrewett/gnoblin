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
//   decorations         decorations, a flag                                       a Titlebar on every client
//   outputs             outputs, stored as written (no native effect yet)          OutputProfile + Display
//   workspaces          workspaces[] with a Layout instance and the gaps           gnoblin.workspaces.add
//   keys                bindings[] of { trigger, action }, actions a closed union  input.primary.bind
//   rules               rules[], a data-only query and its actions                 clients.where(query)
//   on                  listeners[] of { event, handler }                          typed listener sugar
//   setup(gnoblin)      setup, run by the host after the declarative part          the full object API
//
// A key missing from the config adds nothing to the plan. The one exception is `workspaces`: a shell with
// nowhere to put a window is unusable, so no workspace means the baseline's single floating `desktop`.
//
// The host does not apply a user's config alone: it layers it over `defaultConfig` with `mergeConfig`, so a
// config that says nothing about keys still has the stock ones. The rule is in the comment on `mergeConfig`.

import { checkTrigger, parseTrigger } from "gnoblin/bindings";
import type { Binding } from "gnoblin/bindings";
import { WorkspaceSlideBinding } from "gnoblin/gestures";
import type { Gaps, Layout } from "gnoblin/layout";
import { checkGaps, rejectUnknown, resolveLayout } from "gnoblin/workspace";
import type { StockLayoutName } from "gnoblin/workspace";

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

/** Show the workspace at a 1-based position among the shell's workspaces, whatever its id; nothing if there is none. */
export interface ShowPositionAction {
    readonly kind: "show-position";
    readonly position: number;
}

/** Move focus one column left or right when the shown workspace is a `StripLayout`; nothing on any other layout. */
export interface FocusColumnAction {
    readonly kind: "focus-column";
    readonly direction: "left" | "right";
}

/** Bind what `create` makes (a gesture or drag binding) at the key's trigger. */
export interface BindingAction {
    readonly kind: "binding";
    readonly create: () => Binding;
}

/** The closed set of key actions. A closed set can be inspected and diffed; a function cannot. */
export type KeyAction =
    SpawnAction | ShowAction | ShowPositionAction | FocusColumnAction | BindingAction | "close" | KeyHandler;

/** A key action as the plan holds it: `"close"` and functions get a `kind` too. */
export type ResolvedAction =
    | SpawnAction
    | ShowAction
    | ShowPositionAction
    | FocusColumnAction
    | BindingAction
    | { readonly kind: "close" }
    | { readonly kind: "call"; readonly handler: KeyHandler };

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

export type { StockLayoutName };

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
    /** Draws a titlebar on every client (a `Titlebar` canvas under its window). `defaultConfig` turns it on. */
    decorations?: boolean;
    outputs?: Readonly<Record<string, OutputConfig>>;
    workspaces?: readonly WorkspaceConfig[];
    keys?: Readonly<Record<string, KeyAction>>;
    /**
     * The command `Super+space` runs, as a command line or an argv. Nothing is bound when it is absent: the Lua
     * shell's in-shell command entry is not ported, and no stock launcher program is assumed. A key of `keys` for
     * `Super+space` takes precedence.
     */
    launcher?: string | readonly string[];
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
    readonly decorations: boolean;
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

/** `show.position(3)` shows the third workspace, whatever it is called, when there is one. */
function position(index: number): ShowPositionAction {
    if (!Number.isInteger(index) || index < 1 || index > MAX_NUMBERED) {
        throw new RangeError(`show.position needs a whole number from 1 to ${MAX_NUMBERED}, got ${String(index)}`);
    }
    return Object.freeze({ kind: "show-position", position: index });
}

/** `show.positions("Super+{n}")` binds Super+1 to Super+9 to the workspaces at those positions. */
function positions(template: string) {
    if (typeof template !== "string" || !template.includes("{n}")) {
        throw new TypeError(`show.positions needs a template with {n}, such as "Super+{n}"`);
    }
    const keys: Record<string, ShowPositionAction> = {};
    for (let n = 1; n <= MAX_NUMBERED; n++) {
        keys[template.replaceAll("{n}", String(n))] = position(n);
    }
    return keys;
}

export const show = Object.assign(showWorkspace, { numbered, position, positions });

/** `focusColumn("left")`: the focus helper of a strip. */
export function focusColumn(direction: "left" | "right"): FocusColumnAction {
    if (direction !== "left" && direction !== "right") {
        throw new TypeError(`focusColumn needs "left" or "right", got ${String(direction)}`);
    }
    return Object.freeze({ kind: "focus-column", direction });
}

/** `binding(() => new WorkspaceSlideBinding())`: a key whose action is a binding the host makes when it binds. */
export function binding(create: () => Binding): BindingAction {
    if (typeof create !== "function") {
        throw new TypeError("binding needs a function that makes the Binding");
    }
    return Object.freeze({ kind: "binding", create });
}

const TOP_LEVEL_KEYS = [
    "gaps",
    "style",
    "outputs",
    "workspaces",
    "keys",
    "launcher",
    "rules",
    "on",
    "setup",
    "decorations",
];
const LISTENER_EVENTS = ["clientAdded", "clientRemoved", "titleChanged"];
const RULE_KEYS = ["appId", "title", "workspace", "style"];

/** The config the shell runs when the user has none: one floating `desktop` workspace and the stock keys. */
export const defaultConfig: Readonly<GnoblinConfig> = deepFreeze({
    decorations: true,
    workspaces: [{ id: "desktop", layout: "floating" }],
    keys: {
        // The Lua shell's: Super+1 to Super+9 show the workspace at that position, Super+Left and Super+Right move
        // focus between a strip's columns, and a three-finger swipe slides between workspaces.
        ...show.positions("Super+{n}"),
        "Super+Left": focusColumn("left"),
        "Super+Right": focusColumn("right"),
        Swipe3: binding(() => new WorkspaceSlideBinding()),
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

/**
 * `user` laid over `base`, the rule the host applies to `defaultConfig` and the user's `defineConfig`:
 *
 * - `workspaces` and `rules` are lists, and a list the user gives replaces the base's whole. An absent or empty
 *   `workspaces` keeps the base's.
 * - `gaps`, `style` and `outputs` merge by key, the user's value winning; a nested object (a shadow) is one value.
 * - `keys` merge by trigger, compared in canonical spelling, so `"super+1"` replaces the base's `"Super+1"`. A base
 *   key that shows a workspace the merged config does not declare is dropped: it would name nothing.
 * - `on` merges by event, and `setup` is the user's when they give one. `decorations` is not layered: a config that
 *   does not ask for titlebars has none (only the stock config itself turns them on).
 *
 * Pure: neither argument is changed. The user's config is checked for unknown keys here, so a typo is reported
 * against `config` as `desugarConfig` would.
 */
export function mergeConfig(base: GnoblinConfig, user: GnoblinConfig): GnoblinConfig {
    if (user === null || typeof user !== "object") {
        throw new TypeError("a config must be an object");
    }
    rejectUnknown(user, TOP_LEVEL_KEYS, "config");
    const workspaces =
        user.workspaces === undefined || user.workspaces.length === 0 ? base.workspaces : user.workspaces;
    const declared = new Set((workspaces ?? []).map((workspace) => workspace?.id));
    const overridden = new Set(Object.keys(user.keys ?? {}).map((trigger) => parseTrigger(trigger, "a key").text));
    const keys: Record<string, KeyAction> = {};
    for (const [trigger, action] of Object.entries(base.keys ?? {})) {
        const showsMissing =
            action !== null && typeof action === "object" && action.kind === "show" && !declared.has(action.workspace);
        if (!showsMissing && !overridden.has(parseTrigger(trigger, "a key").text)) {
            keys[trigger] = action;
        }
    }
    Object.assign(keys, user.keys);
    const rules = user.rules ?? base.rules;
    const setup = user.setup ?? base.setup;
    const launcher = user.launcher ?? base.launcher;
    return {
        gaps: { ...base.gaps, ...user.gaps },
        style: { ...base.style, ...user.style },
        outputs: { ...base.outputs, ...user.outputs },
        keys,
        on: { ...base.on, ...user.on },
        ...(workspaces === undefined ? {} : { workspaces }),
        ...(rules === undefined ? {} : { rules }),
        ...(setup === undefined ? {} : { setup }),
        ...(user.decorations === undefined ? {} : { decorations: user.decorations }),
        ...(launcher === undefined ? {} : { launcher }),
    };
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
        decorations: checkDecorations(config.decorations),
        style: config.style ?? {},
        outputs: config.outputs ?? {},
        workspaces,
        bindings: [...planKeys(config.keys ?? {}, ids), ...planLauncher(config.launcher, config.keys ?? {})],
        rules: planRules(config.rules ?? [], ids),
        listeners: planListeners(config.on ?? {}),
        setup: checkSetup(config.setup),
    };
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

function planKeys(keys: Readonly<Record<string, KeyAction>>, workspaces: ReadonlySet<string>): PlannedKey[] {
    return Object.entries(keys).map(([trigger, action]) => {
        checkTrigger(trigger, "a key");
        return { trigger, action: resolveAction(action, `keys["${trigger}"]`, workspaces) };
    });
}

/** The key the launcher command is bound to. */
const LAUNCHER_TRIGGER = "Super+space";

/** `Super+space` running the launcher command, unless `keys` binds that trigger itself or there is no launcher. */
function planLauncher(launcher: GnoblinConfig["launcher"], keys: Readonly<Record<string, KeyAction>>): PlannedKey[] {
    if (launcher === undefined) {
        return [];
    }
    if (typeof launcher !== "string" && !Array.isArray(launcher)) {
        throw new TypeError("config.launcher must be a command line or an argv array");
    }
    const wanted = parseTrigger(LAUNCHER_TRIGGER, "the launcher key").text;
    if (Object.keys(keys).some((trigger) => parseTrigger(trigger, "a key").text === wanted)) {
        return [];
    }
    const command = typeof launcher === "string" ? launcher.trim().split(/\s+/) : launcher;
    try {
        return [{ trigger: LAUNCHER_TRIGGER, action: spawn(command) }];
    } catch {
        throw new TypeError("config.launcher must be a non-empty command line or an argv of strings");
    }
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
        if (action.kind === "show-position" && Number.isInteger(action.position)) {
            return action;
        }
        if (action.kind === "focus-column" && (action.direction === "left" || action.direction === "right")) {
            return action;
        }
        if (action.kind === "binding" && typeof action.create === "function") {
            return action;
        }
    }
    throw new TypeError(
        `${where} must be spawn(...), show(...), show.position(...), focusColumn(...), binding(...), "close" or a function`,
    );
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

function checkDecorations(decorations: GnoblinConfig["decorations"]): boolean {
    if (decorations !== undefined && typeof decorations !== "boolean") {
        throw new TypeError("config.decorations: must be true or false");
    }
    return decorations ?? false;
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
