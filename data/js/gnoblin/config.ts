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
//   rules               rules[], compiled from data: a query and its actions        a RuleEngine on gnoblin.clients
//   focus               focus, the FocusConfig as written (settings the seat lacks)  a FocusPolicy on gnoblin.clients
//   edgeTiling          edgeTiling, a flag that overrides the seat's edge_tiling     DragEvent.edgeTiling
//   dragConstraint      dragConstraint, overrides the seat's drag_constraint         DragEvent.dragConstraint
//   on                  listeners[] of { event, handler }                          typed listener sugar
//   setup(gnoblin)      setup, run by the host after the declarative part          the full object API
//   dialogs, appearance, cursor, input, ...  document, a patch of the config document   gnoblin.config commit
//
// The document sections (`DOCUMENT_SECTIONS`) are the settings of the config document the compositor holds, in
// camelCase (`windowManagement`, `buttonLayout`); the plan carries them as the document writes them
// (`window-management`, `button-layout`) and the host lays that patch over the document in force. The table of
// what each `defineConfig` key becomes is in design/js-runtime.md.
//
// A key missing from the config adds nothing to the plan. The one exception is `workspaces`: a shell with
// nowhere to put a window is unusable, so no workspace means the baseline's single floating `desktop`.
//
// The host does not apply a user's config alone: it layers it over `defaultConfig` with `mergeConfig`, so a
// config that says nothing about keys still has the stock ones. The rule is in the comment on `mergeConfig`.

import { checkTrigger, dragConstraintMode, parseTrigger } from "gnoblin/bindings";
import type { Binding, DragConstraint } from "gnoblin/bindings";
import { focusSettings } from "gnoblin/focus";
import type { FocusConfig } from "gnoblin/focus";
import { WorkspaceSlideBinding } from "gnoblin/gestures";
import type { Gaps, Layout } from "gnoblin/layout";
import { compileRules } from "gnoblin/rules";
import type { RuleConfig } from "gnoblin/rules";
import { checkGaps, rejectUnknown, resolveLayout } from "gnoblin/workspace";
import type { StockLayoutName } from "gnoblin/workspace";
import type { Client } from "gnoblin/host";

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

export interface ConfigListeners {
    clientAdded?(client: Client): void;
    clientRemoved?(client: Client): void;
    titleChanged?(client: Client, title: string): void;
    focusChanged?(client: Client, focused: boolean): void;
}

/** A value of the config document: what a setting holds. */
export type DocumentValue =
    string | number | boolean | readonly DocumentValue[] | { readonly [key: string]: DocumentValue };

/** One section of the config document, as a table of settings. Keys are camelCase or the document's kebab-case. */
export interface DocumentSection {
    readonly [setting: string]: DocumentValue | undefined;
}

/** The look of the compositor's own consent and passphrase dialogs (`dialogs.style` in the document). */
export interface DialogStyleConfig {
    /** The corner radius in points; a dialog is never rounder than 24. */
    radius?: number;
    /** A named accent, `#rgb` or `#rrggbb`. */
    accent?: string;
    font?: { family?: string; size?: number };
    buttonLayout?: "start" | "end" | "stretch" | "stacked";
    /** A number from 0.1 to 100; a dialog is drawn at 1 to 3. */
    scale?: number;
}

export interface DialogsConfig {
    style?: DialogStyleConfig;
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
    /** Window rules: the first matching rule wins, `all: true` lets the next one apply too. See `gnoblin/rules`. */
    rules?: readonly RuleConfig[];
    /** Focus settings. A key left out is the seat's (the compositor's `window_management` config). */
    focus?: FocusConfig;
    /** Whether dropping a dragged window on a screen edge tiles it. Left out, the seat's `edge_tiling`. */
    edgeTiling?: boolean;
    /** Where a dragged window may go. Left out, the seat's `drag_constraint`. */
    dragConstraint?: DragConstraint;
    on?: ConfigListeners;
    /** Runs after the declarative part and may use any class of the object API. */
    setup?(gnoblin: GnoblinHost): void;
    /** The compositor's own dialogs. Committed to the config document. */
    dialogs?: DialogsConfig;
    /** Committed to the config document: the settings of the matching section. */
    compositor?: DocumentSection;
    windowManagement?: DocumentSection;
    cursor?: DocumentSection;
    appearance?: DocumentSection;
    input?: DocumentSection;
    inputSources?: DocumentSection;
    accessibility?: DocumentSection;
    layerShell?: DocumentSection;
    keyring?: DocumentSection;
    auth?: DocumentSection;
    location?: DocumentSection;
    compat?: DocumentSection;
    capture?: DocumentSection;
    monitors?: DocumentSection;
    xwayland?: DocumentSection;
    logging?: DocumentSection;
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

/** The document patch a config makes: each section it sets, with the keys as the document spells them. */
export type DocumentPatch = { readonly [section: string]: DocumentSection };

export interface ConfigPlan {
    readonly gaps: Gaps;
    readonly style: StyleConfig;
    readonly decorations: boolean;
    readonly outputs: Readonly<Record<string, OutputConfig>>;
    readonly workspaces: readonly PlannedWorkspace[];
    readonly bindings: readonly PlannedKey[];
    readonly rules: readonly RuleConfig[];
    /** The `focus` keys the config set, checked, and no others. */
    readonly focus: FocusConfig;
    /** `null` when the config says nothing, so the seat's value stands. */
    readonly edgeTiling: boolean | null;
    readonly dragConstraint: DragConstraint | null;
    readonly listeners: readonly PlannedListener[];
    readonly setup: ((gnoblin: GnoblinHost) => void) | undefined;
    readonly document: DocumentPatch;
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

/**
 * The sections of the config document a `defineConfig` key may set: the key as written in a config and the
 * section as the document names it. Sections that are lists or tables keyed by the user's own names
 * (`workspaces`, `autostart`, `shortcuts`, `protocols`, `touchpad-gestures`) and the ones only carried
 * (`animations`, `keybindings`, `portals`) are not here: a camelCase key cannot be turned back into a name the
 * user chose.
 */
export const DOCUMENT_SECTIONS = [
    ["compositor", "compositor"],
    ["windowManagement", "window-management"],
    ["cursor", "cursor"],
    ["appearance", "appearance"],
    ["dialogs", "dialogs"],
    ["input", "input"],
    ["inputSources", "input-sources"],
    ["accessibility", "accessibility"],
    ["layerShell", "layer-shell"],
    ["keyring", "keyring"],
    ["auth", "auth"],
    ["location", "location"],
    ["compat", "compat"],
    ["capture", "capture"],
    ["monitors", "monitors"],
    ["xwayland", "xwayland"],
    ["logging", "logging"],
] as const;

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
    "focus",
    "edgeTiling",
    "dragConstraint",
    ...DOCUMENT_SECTIONS.map(([name]) => name),
];
const LISTENER_EVENTS = ["clientAdded", "clientRemoved", "titleChanged", "focusChanged"];

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
 * - `focus` merges by key; `edgeTiling` and `dragConstraint` are the user's when they give one.
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
    const sections: { [name: string]: object } = {};
    for (const [name] of DOCUMENT_SECTIONS) {
        const lower = base[name];
        const higher = user[name];
        if (lower !== undefined || higher !== undefined) {
            sections[name] =
                lower === undefined || higher === undefined ? (lower ?? higher ?? {}) : mergeTables(lower, higher);
        }
    }
    const launcher = user.launcher ?? base.launcher;
    // The focus keys merge by key, so a config that sets one keeps the base's others; the other two are single values.
    const focus = base.focus === undefined && user.focus === undefined ? undefined : { ...base.focus, ...user.focus };
    const edgeTiling = user.edgeTiling ?? base.edgeTiling;
    const dragConstraint = user.dragConstraint ?? base.dragConstraint;
    return Object.assign(
        {
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
            ...(focus === undefined ? {} : { focus }),
            ...(edgeTiling === undefined ? {} : { edgeTiling }),
            ...(dragConstraint === undefined ? {} : { dragConstraint }),
        },
        sections,
    );
}

function isTable(value: unknown): value is object {
    return value !== null && typeof value === "object" && !Array.isArray(value);
}

/** `user` laid over `base` key by key, one level of table at a time; a list or a scalar of the user's replaces. */
function mergeTables<T extends object>(base: T, user: T): T {
    const merged: Record<string, unknown> = { ...base };
    for (const [key, value] of Object.entries(user)) {
        const before = merged[key];
        merged[key] = isTable(before) && isTable(value) ? mergeTables(before, value) : value;
    }
    // The keys are the two tables' own, so the result has the shape both have.
    return merged as T;
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
        focus: planFocus(config.focus),
        edgeTiling: checkEdgeTiling(config.edgeTiling),
        dragConstraint: checkDragConstraint(config.dragConstraint),
        listeners: planListeners(config.on ?? {}),
        setup: checkSetup(config.setup),
        document: planDocument(config),
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
    // Compiling is the check: it throws naming the rule's index and the field. The host compiles again to run them.
    compileRules(rules, workspaces);
    return [...rules];
}

/** The keys of `focus` the config set. Checked against the documented defaults, so a bad value fails the stage. */
function planFocus(focus: FocusConfig | undefined): FocusConfig {
    if (focus === undefined) {
        return {};
    }
    focusSettings(focus, "config.focus");
    return Object.fromEntries(Object.entries(focus).filter(([, value]) => value !== undefined));
}

function checkEdgeTiling(edgeTiling: GnoblinConfig["edgeTiling"]): boolean | null {
    if (edgeTiling !== undefined && typeof edgeTiling !== "boolean") {
        throw new TypeError("config.edgeTiling: must be true or false");
    }
    return edgeTiling ?? null;
}

function checkDragConstraint(constraint: GnoblinConfig["dragConstraint"]): DragConstraint | null {
    return constraint === undefined ? null : dragConstraintMode(constraint);
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

/** One value of the document, checked to be something the document can hold, with its keys in the document's spelling. */
function documentValue(value: unknown, where: string): DocumentValue {
    if (typeof value === "string" || typeof value === "boolean") {
        return value;
    }
    if (typeof value === "number") {
        if (!Number.isFinite(value)) {
            throw new RangeError(`${where} must be a finite number`);
        }
        return value;
    }
    if (Array.isArray(value)) {
        return value.map((item, i) => documentValue(item, `${where}[${i}]`));
    }
    if (isTable(value)) {
        return documentTable(value, where);
    }
    throw new TypeError(`${where} must be a string, a number, a boolean, a list or a table of settings`);
}

/** A table of settings with its keys in the document's spelling: `buttonLayout` is `button-layout`. */
function documentTable(table: object, where: string): { [setting: string]: DocumentValue } {
    const settings: { [setting: string]: DocumentValue } = {};
    for (const [written, value] of Object.entries(table)) {
        if (value === undefined) {
            continue;
        }
        // A key already in the document's spelling is left alone.
        const key = written.replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase();
        if (key in settings) {
            throw new TypeError(`${where}.${written} sets "${key}" twice`);
        }
        settings[key] = documentValue(value, `${where}.${written}`);
    }
    return settings;
}

/** The document patch `config` makes: only the sections it sets. */
function planDocument(config: GnoblinConfig): DocumentPatch {
    const patch: { [section: string]: DocumentSection } = {};
    for (const [name, section] of DOCUMENT_SECTIONS) {
        const settings = config[name];
        if (settings === undefined) {
            continue;
        }
        if (!isTable(settings)) {
            throw new TypeError(`config.${name} must be a table of settings`);
        }
        patch[section] = documentTable(settings, `config.${name}`);
    }
    return patch;
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
