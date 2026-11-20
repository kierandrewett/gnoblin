import type { Init } from "gnoblin-std/emitted/native";
import { Rect, Split, ClientState, OutputInfo, Span, Areas, Event, QueryTerm, ProfileSpec, DisplaySpec, DisplayInfo, Placement, LayoutQuery, FloatingQuery, LayoutSpec, WorkspaceSpec, WorkspaceInfo, Show, OutputPolicy, WorkspacePolicy, Option, RingSlot, DragEvent, GestureEvent, BindingSpec, DragStep, DragOutcome, Stats, Prop, TransitionOptions, TransitionInfo, StyleEntry } from "gnoblin-std/emitted/records";
export declare class Collection {
    readonly handle: number;
    constructor(handle: number);
    static root(kind: number): Collection | null;
    where(terms: Init<QueryTerm>[]): Collection | null;
    kind(): number;
    len(): number;
    members(): number[];
    contains(member: number): boolean;
    listen(event: number, flags: number, cb: (event: Event) => unknown): Registration;
    add(member: number): number;
    remove(member: number): boolean;
    clear(): number;
    swap(a: number, b: number): number;
}
export declare class Client {
    readonly node: number;
    constructor(node: number);
    workspace(): Workspace | null;
    moveToWorkspace(workspace: Workspace | number): boolean;
    setFloating(floating: number): number;
    isFloating(): boolean;
    state(): ClientState;
    setState(state: Init<ClientState>): number;
    snap(zone: number, area: Init<Rect>, edgeTiling: number): boolean;
    unsnap(forget: number): boolean;
    areas(): Areas;
    restoreSnap(zone: number, floating: Init<Rect>): number;
}
export declare class Workspace {
    readonly handle: number;
    constructor(handle: number);
    static new(spec: Init<WorkspaceSpec>): Workspace | null;
    static byId(id: string): Workspace | null;
    info(): WorkspaceInfo;
    add(): number;
    remove(): boolean;
    setName(name: string): number;
    setGaps(inner: number, outer: number): number;
    setLayout(spec: Init<LayoutSpec>): number;
    invalidate(): number;
    clients(): Collection | null;
    show(show: Init<Show>): number;
    walk(step: number): number;
    findSplit(client: number, x: number, y: number): Split | null;
    setSplit(token: number, ratio: number, crossRatio: number): number;
    resetSplit(token: number): number;
    setLayoutKind(kind: number, options: Init<Option>[]): number;
    layoutSet(option: Init<Option>): number;
    layoutGet(name: string): Option;
    layoutAct(action: string, args: Init<Option>[]): Option;
    layoutKind(): number;
    restoreClient(client: number, arrived: number): boolean;
}
export declare class Policy {
    static outputs(policy: Init<OutputPolicy>): number;
    static workspaces(policy: Init<WorkspacePolicy>): number;
}
export declare class Output {
    readonly node: number;
    constructor(node: number);
    workspace(): Workspace | null;
    info(): OutputInfo;
    setInsets(top: number, bottom: number): number;
    static span(span: Init<Span>): number;
}
export declare class Easing {
    static sample(easing: number, t: number): number;
}
export declare class Transition {
    readonly handle: number;
    constructor(handle: number);
    static begin(options: Init<TransitionOptions>): number;
    static set(node: number, key: string, value: Init<Prop>): number;
    static get(node: number, key: string): Prop;
    static commit(): Transition | null;
    static abort(): number;
    static animate(node: number, key: string, value: Init<Prop>, options: Init<TransitionOptions>): Transition | null;
    control(op: number, value: number): number;
    info(): TransitionInfo;
    finished(cb: (event: Event) => unknown): Registration;
    static release(node: number, key: string): number;
    /** `transition_begin`, then `body()`, then `transition_commit`; `transition_abort` when `body` throws. */
    static run(options: Init<TransitionOptions>, body: () => void): Transition | null;
}
export declare class Style {
    readonly handle: number;
    constructor(handle: number);
    static scope(scope: number, target: number): Style | null;
    set(prop: number, value: Init<Prop>): number;
    setList(prop: number, entries: Init<StyleEntry>[]): number;
    unset(prop: number): number;
    get(prop: number): Prop;
    list(prop: number): StyleEntry[];
    static computed(client: number, prop: number): Prop;
    static computedList(client: number, prop: number): StyleEntry[];
    static origin(client: number, prop: number): string;
    static notes(client: number): string;
    rank(rank: number): number;
    static reset(): number;
}
export declare class Scene {
    static new(kind: number): number | null;
    static destroy(node: number): number;
    static insert(parent: number, node: number, index: number): number;
    static root(): number | null;
    static set(node: number, key: string, value: Init<Prop>): number;
    static get(node: number, key: string): Prop;
    static children(node: number): number[];
    static parent(node: number): number | null;
}
export declare class Layout {
    static ring(count: number, focus: number, area: Init<Rect>, options: Init<Option>[]): RingSlot[];
}
export declare class Binding {
    static drag(spec: Init<BindingSpec>, cb: (event: DragEvent) => unknown): Registration;
    static gesture(spec: Init<BindingSpec>, cb: (event: GestureEvent) => unknown): Registration;
    static builtin(kind: number, options: Init<Option>[]): Registration;
}
export declare class Builtin {
    readonly handle: number;
    constructor(handle: number);
    static new(kind: number, options: Init<Option>[]): Builtin | null;
    free(): number;
    set(option: Init<Option>): number;
    get(name: string): Option;
    compute(query: Init<LayoutQuery>): Placement[];
    act(action: string, values: Init<Option>[], query: Init<LayoutQuery>): [number | null, Option];
    clientRemoved(query: Init<LayoutQuery>, client: number): number | null;
    clientsSwapped(a: number, b: number): number;
    findSplit(client: number, x: number, y: number): Split | null;
    setSplit(token: number, ratio: number, crossRatio: number): number;
    resetSplit(token: number): number;
}
export declare class Drag {
    static step(kind: number, options: Init<Option>[], step: Init<DragStep>): DragOutcome;
    static clamp(pos: number, size: number, start: number, span: number, constraint: number): number;
    static resize(start: Init<Rect>, sides: number, dx: number, dy: number, limits: Init<ClientState>): Rect;
    static zone(x: number, y: number, area: Init<Rect>, margin: number): number;
    static tile(zone: number, area: Init<Rect>): Rect | null;
}
export declare class Gesture {
    static way(kind: number, dx: number, dy: number, scale: number, threshold: number): number;
}
export declare class Floating {
    static place(query: Init<FloatingQuery>): Rect;
}
export declare class Profile {
    readonly handle: number;
    constructor(handle: number);
    static new(spec: Init<ProfileSpec>): Profile | null;
    set(spec: Init<ProfileSpec>): number;
    add(): number;
    remove(): boolean;
    static list(): number[];
}
export declare class Display {
    readonly handle: number;
    constructor(handle: number);
    static new(spec: Init<DisplaySpec>): Display | null;
    setMode(mode: number): number;
    add(): number;
    remove(): boolean;
    replace(next: Display | number): number;
    static clear(): number;
    static list(): number[];
    info(): DisplayInfo;
    outputs(): number[];
    static at(x: number, y: number): Display | null;
    assign(workspace: Workspace | number): number;
    static snapshot(): number;
    static restore(token: number): number;
    static reflow(): number;
}
export declare class Registration {
    readonly handle: number;
    constructor(handle: number);
    remove(): number;
}
export declare function open(): number;
export declare function stats(): Stats;
export declare function remove(reg: Registration | number): number;
