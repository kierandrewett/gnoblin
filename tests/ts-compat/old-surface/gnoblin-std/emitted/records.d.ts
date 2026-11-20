import type { Init } from "gnoblin-std/emitted/native";
import { EventKind, TermKind, LayoutKind, SelectKind, PropKind, EntryKind, ValueKind } from "gnoblin-std/emitted/constants";
export interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}
export declare function rectToNative(value: Init<Rect>): Record<string, unknown>;
export declare function rectFromNative(data: any): Rect;
export interface Split {
    flags: number;
    token: number;
    axis: number;
    direction: number;
    span: number;
    min: number;
    max: number;
    ratio: number;
    crossAxis: number;
    crossDirection: number;
    crossSpan: number;
    crossMin: number;
    crossMax: number;
    crossRatio: number;
}
export declare function splitToNative(value: Init<Split>): Record<string, unknown>;
export declare function splitFromNative(data: any): Split;
export interface ClientState {
    flags: number;
    insetTop: number;
    insetRight: number;
    insetBottom: number;
    insetLeft: number;
    minWidth: number;
    minHeight: number;
    maxWidth: number;
    maxHeight: number;
    floating: Rect;
    snapZone: number;
    snapFloating: Rect;
    fittedTop: number;
    fittedRight: number;
    fittedBottom: number;
    fittedLeft: number;
}
export declare function clientStateToNative(value: Init<ClientState>): Record<string, unknown>;
export declare function clientStateFromNative(data: any): ClientState;
export interface OutputInfo {
    flags: number;
    workspace: number;
    rect: Rect;
    area: Rect;
}
export declare function outputInfoToNative(value: Init<OutputInfo>): Record<string, unknown>;
export declare function outputInfoFromNative(data: any): OutputInfo;
export interface Span {
    lead: number;
    members: number[];
}
export declare function spanToNative(value: Init<Span>): Record<string, unknown>;
export declare function spanFromNative(data: any): Span;
export interface Areas {
    flags: number;
    output: Rect;
    area: Rect;
}
export declare function areasToNative(value: Init<Areas>): Record<string, unknown>;
export declare function areasFromNative(data: any): Areas;
export interface Event {
    kind: EventKind;
    flag: number;
    collection: number;
    member: number;
    text: string;
    rect: Rect;
}
export declare function eventToNative(value: Init<Event>): Record<string, unknown>;
export declare function eventFromNative(data: any): Event;
/** The kinds of `Event`, each with the fields it reads. */
export declare const Event: {
    added(): Event;
    removed(): Event;
    titleChanged(text?: string): Event;
    appIdChanged(text?: string): Event;
    focusChanged(flag?: number): Event;
    geometryChanged(rect?: Rect): Event;
    closed(): Event;
    ready(): Event;
    decorationChanged(text?: string): Event;
    modeChanged(text?: string): Event;
    transitionFinished(member?: number, flag?: number): Event;
    displays(member?: number): Event;
    applied(member?: number, text?: string): Event;
    note(text?: string): Event;
};
export interface QueryTerm {
    kind: TermKind;
    flag: number;
    key: string;
    text: string;
    number: number;
}
export declare function queryTermToNative(value: Init<QueryTerm>): Record<string, unknown>;
export declare function queryTermFromNative(data: any): QueryTerm;
/** The kinds of `QueryTerm`, each with the fields it reads. */
export declare const QueryTerm: {
    text(text?: string): QueryTerm;
    number(number?: number): QueryTerm;
    bool(flag?: number): QueryTerm;
};
export interface ProfileSpec {
    flags: number;
    id: string;
    connector: string;
    make: string;
    model: string;
    serial: string;
    mode: string;
    transform: string;
    scale: number;
    supersample: number;
    position: number;
    align: number;
    x: number;
    y: number;
    targetKind: number;
    target: number;
}
export declare function profileSpecToNative(value: Init<ProfileSpec>): Record<string, unknown>;
export declare function profileSpecFromNative(data: any): ProfileSpec;
export interface Selector {
    kind: SelectKind;
    target: number;
}
export declare function selectorToNative(value: Init<Selector>): Record<string, unknown>;
export declare function selectorFromNative(data: any): Selector;
/** The kinds of `Selector`, each with the fields it reads. */
export declare const Selector: {
    profile(): Selector;
    output(): Selector;
};
export interface DisplaySpec {
    mode: number;
    id: string;
    selectors: Selector[];
    wanted: number;
}
export declare function displaySpecToNative(value: Init<DisplaySpec>): Record<string, unknown>;
export declare function displaySpecFromNative(data: any): DisplaySpec;
export interface DisplayInfo {
    flags: number;
    mode: number;
    id: string;
    shown: number;
    content: number;
    wanted: number;
}
export declare function displayInfoToNative(value: Init<DisplayInfo>): Record<string, unknown>;
export declare function displayInfoFromNative(data: any): DisplayInfo;
export interface Placement {
    flags: number;
    node: number;
    bounds: Rect;
    z: number;
    rotationX: number;
    rotationY: number;
    rotationZ: number;
    scaleX: number;
    scaleY: number;
    scaleZ: number;
    offsetX: number;
    offsetY: number;
}
export declare function placementToNative(value: Init<Placement>): Record<string, unknown>;
export declare function placementFromNative(data: any): Placement;
export interface LayoutContext {
    workspace: number;
    children: number[];
    area: Rect;
    gapInner: number;
    gapOuter: number;
    focused: number;
}
export declare function layoutContextToNative(value: Init<LayoutContext>): Record<string, unknown>;
export declare function layoutContextFromNative(data: any): LayoutContext;
export interface LayoutQuery {
    flags: number;
    children: number[];
    area: Rect;
    gapInner: number;
    gapOuter: number;
    focused: number;
    pointerX: number;
    pointerY: number;
    focusedWidth: number;
    focusedHeight: number;
}
export declare function layoutQueryToNative(value: Init<LayoutQuery>): Record<string, unknown>;
export declare function layoutQueryFromNative(data: any): LayoutQuery;
export interface FloatingQuery {
    flags: number;
    mode: number;
    step: number;
    width: number;
    height: number;
    area: Rect;
    occupied: Rect[];
}
export declare function floatingQueryToNative(value: Init<FloatingQuery>): Record<string, unknown>;
export declare function floatingQueryFromNative(data: any): FloatingQuery;
export interface LayoutSpec {
    kind: LayoutKind;
    placement: number;
    cascadeStep: number;
    vtable: Layout | null;
}
export declare function layoutSpecToNative(value: Init<LayoutSpec>): Record<string, unknown>;
export declare function layoutSpecFromNative(data: any): LayoutSpec;
/** The kinds of `LayoutSpec`, each with the fields it reads. */
export declare const LayoutSpec: {
    floating(): LayoutSpec;
    foreign(vtable?: Layout | null): LayoutSpec;
};
export interface WorkspaceSpec {
    id: string;
    name: string;
    gapInner: number;
    gapOuter: number;
    layout: LayoutSpec | null;
}
export declare function workspaceSpecToNative(value: Init<WorkspaceSpec>): Record<string, unknown>;
export declare function workspaceSpecFromNative(data: any): WorkspaceSpec;
export interface WorkspaceInfo {
    attached: number;
    id: string;
    name: string;
    index: number;
    active: number;
    output: number;
    gapInner: number;
    gapOuter: number;
}
export declare function workspaceInfoToNative(value: Init<WorkspaceInfo>): Record<string, unknown>;
export declare function workspaceInfoFromNative(data: any): WorkspaceInfo;
export interface Show {
    durationMs: number;
    progress: number;
    output: number;
}
export declare function showToNative(value: Init<Show>): Record<string, unknown>;
export declare function showFromNative(data: any): Show;
export interface OutputPolicy {
    mode: number;
    lead: number;
}
export declare function outputPolicyToNative(value: Init<OutputPolicy>): Record<string, unknown>;
export declare function outputPolicyFromNative(data: any): OutputPolicy;
export interface WorkspacePolicy {
    mode: number;
    namePrefix: string;
    gapInner: number;
    gapOuter: number;
    layout: LayoutSpec | null;
}
export declare function workspacePolicyToNative(value: Init<WorkspacePolicy>): Record<string, unknown>;
export declare function workspacePolicyFromNative(data: any): WorkspacePolicy;
export interface Option {
    kind: ValueKind;
    flag: number;
    name: string;
    text: string;
    number: number;
    node: number;
}
export declare function optionToNative(value: Init<Option>): Record<string, unknown>;
export declare function optionFromNative(data: any): Option;
/** The kinds of `Option`, each with the fields it reads. */
export declare const Option: {
    none(): Option;
    bool(flag?: number): Option;
    number(number?: number): Option;
    enum_(number?: number): Option;
    node(node?: number): Option;
    text(text?: string): Option;
};
export interface RingSlot {
    rect: Rect;
    opacity: number;
    rotation: number;
    z: number;
}
export declare function ringSlotToNative(value: Init<RingSlot>): Record<string, unknown>;
export declare function ringSlotFromNative(data: any): RingSlot;
export interface DragEvent {
    phase: number;
    flags: number;
    constraint: number;
    button: number;
    client: number;
    x: number;
    y: number;
    originX: number;
    originY: number;
    localX: number;
    localY: number;
    workArea: Rect;
}
export declare function dragEventToNative(value: Init<DragEvent>): Record<string, unknown>;
export declare function dragEventFromNative(data: any): DragEvent;
export interface GestureEvent {
    phase: number;
    flags: number;
    kind: number;
    fingers: number;
    direction: number;
    way: number;
    client: number;
    dx: number;
    dy: number;
    scale: number;
    rotation: number;
    progress: number;
    x: number;
    y: number;
}
export declare function gestureEventToNative(value: Init<GestureEvent>): Record<string, unknown>;
export declare function gestureEventFromNative(data: any): GestureEvent;
export interface BindingSpec {
    trigger: string;
    priority: number;
    threshold: number;
}
export declare function bindingSpecToNative(value: Init<BindingSpec>): Record<string, unknown>;
export declare function bindingSpecFromNative(data: any): BindingSpec;
export interface DragTile {
    flags: number;
    id: number;
    rect: Rect;
}
export declare function dragTileToNative(value: Init<DragTile>): Record<string, unknown>;
export declare function dragTileFromNative(data: any): DragTile;
export interface DragStep {
    clientFlags: number;
    event: DragEvent | null;
    bounds: Rect;
    start: Rect;
    sides: number;
    state: ClientState | null;
    split: Split | null;
    tiles: DragTile[];
}
export declare function dragStepToNative(value: Init<DragStep>): Record<string, unknown>;
export declare function dragStepFromNative(data: any): DragStep;
export interface DragOutcome {
    flags: number;
    snapZone: number;
    bounds: Rect;
    floating: Rect;
    swapWith: number;
    sides: number;
    reserved2: number;
    ratio: number;
    crossRatio: number;
}
export declare function dragOutcomeToNative(value: Init<DragOutcome>): Record<string, unknown>;
export declare function dragOutcomeFromNative(data: any): DragOutcome;
export interface Stats {
    workspaces: number;
    clients: number;
    collections: number;
    registrations: number;
    layoutPasses: number;
    computes: number;
}
export declare function statsToNative(value: Init<Stats>): Record<string, unknown>;
export declare function statsFromNative(data: any): Stats;
export interface Prop {
    kind: PropKind;
    a: number;
    b: number;
    c: number;
    d: number;
    text: string;
    node: number;
}
export declare function propToNative(value: Init<Prop>): Record<string, unknown>;
export declare function propFromNative(data: any): Prop;
/** The kinds of `Prop`, each with the fields it reads. */
export declare const Prop: {
    unset(): Prop;
    number(a?: number): Prop;
    bool(a?: number): Prop;
    triple(a?: number, b?: number, c?: number): Prop;
    text(text?: string): Prop;
    corners(a?: number, b?: number, c?: number, d?: number): Prop;
    null_(): Prop;
    node(node?: number): Prop;
};
export interface TransitionOptions {
    autoplay: number;
    easing: number;
    durationMs: number;
    delayMs: number;
}
export declare function transitionOptionsToNative(value: Init<TransitionOptions>): Record<string, unknown>;
export declare function transitionOptionsFromNative(data: any): TransitionOptions;
export interface TransitionInfo {
    state: number;
    id: number;
    durationMs: number;
    progress: number;
    paused: number;
    reversed: number;
}
export declare function transitionInfoToNative(value: Init<TransitionInfo>): Record<string, unknown>;
export declare function transitionInfoFromNative(data: any): TransitionInfo;
export interface StyleEntry {
    kind: EntryKind;
    inset: number;
    a: number;
    b: number;
    c: number;
    d: number;
    color: string;
    source: string;
    params: number[];
}
export declare function styleEntryToNative(value: Init<StyleEntry>): Record<string, unknown>;
export declare function styleEntryFromNative(data: any): StyleEntry;
/** The kinds of `StyleEntry`, each with the fields it reads. */
export declare const StyleEntry: {
    border(a?: number, b?: number, color?: string, inset?: number): StyleEntry;
    shadow(a?: number, b?: number, c?: number, d?: number, color?: string, inset?: number): StyleEntry;
    blur(a?: number): StyleEntry;
    saturate(a?: number): StyleEntry;
    brightness(a?: number): StyleEntry;
    contrast(a?: number): StyleEntry;
    shader(source?: string, params?: number[]): StyleEntry;
};
export interface Layout {
    compute(cx: LayoutContext): Init<Placement>[];
    clientAdded?(cx: LayoutContext, client: number): void;
    clientRemoved?(cx: LayoutContext, client: number): void;
    areaChanged?(old: Rect, new_: Rect): void;
    clientsSwapped?(a: number, b: number): void;
    revision?(): number;
    findSplit?(client: number, x: number, y: number): Init<Split> | null;
    setSplit?(token: number, ratio: number, crossRatio: number): void;
    resetSplit?(token: number): void;
}
export declare function layoutToNative(target: Layout): Record<string, unknown>;
