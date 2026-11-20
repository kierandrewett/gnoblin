import { Registration, Workspace } from "gnoblin-std/emitted/classes";
import type { DragOutcome, DragStep } from "gnoblin-std/emitted/records";
import type { Init } from "gnoblin-std/emitted/native";
type Shapes = Record<string, [number, string, boolean]>;
/** A built-in layout: options before it is attached are kept here, after it they are the workspace's. */
export declare class BuiltinLayout {
    readonly kind: number;
    private readonly shapes;
    private readonly actionShapes;
    private options;
    workspace: Workspace | null;
    protected constructor(kind: number, shapes: Shapes, actionShapes: Shapes, options: Record<string, unknown>);
    /** Gives the workspace this layout; its options and actions act on that workspace from then on. */
    attach(workspace: Workspace): this;
    protected getOption(name: string): unknown;
    protected setOption(name: string, value: unknown): void;
    protected act(name: string, args: [string, unknown][]): unknown;
}
/** A pointer binding the library runs itself. */
export declare class BuiltinBinding {
    readonly kind: number;
    private readonly shapes;
    private readonly options;
    protected constructor(kind: number, shapes: Shapes, options: Record<string, unknown>);
    /** Claims the binding's trigger; `remove()` on the result ends it. */
    start(): Registration;
    /**
     * Gives the binding's rules one step of a drag the shell matched itself (`drag_step`): nothing is claimed, the chord, the press
     * and the threshold are the shell's, and so is what a drag keeps between its steps. The answer says what to write to the shell's
     * own window, tile and divider objects.
     */
    step(step: Init<DragStep>): DragOutcome;
    private list;
}
export declare const MasterStackOrientation: {
    readonly LEFT: 0;
    readonly RIGHT: 1;
    readonly TOP: 2;
    readonly BOTTOM: 3;
};
export type MasterStackOrientation = (typeof MasterStackOrientation)[keyof typeof MasterStackOrientation];
export interface MasterStackLayoutOptions {
    ratio?: number;
    masterCount?: number;
    gap?: number | null;
    orientation?: MasterStackOrientation;
}
export declare class MasterStackLayout extends BuiltinLayout {
    static readonly KIND = 1;
    static readonly SHAPES: Record<string, [number, string, boolean]>;
    static readonly ACTION_SHAPES: Record<string, [number, string, boolean]>;
    constructor(options?: MasterStackLayoutOptions);
    get ratio(): number;
    set ratio(value: number);
    get masterCount(): number;
    set masterCount(value: number);
    get gap(): number | null;
    set gap(value: number | null);
    get orientation(): MasterStackOrientation;
    set orientation(value: MasterStackOrientation);
}
export declare const GridFill: {
    readonly ROW: 0;
    readonly COLUMN: 1;
};
export type GridFill = (typeof GridFill)[keyof typeof GridFill];
export interface GridLayoutOptions {
    columns?: number;
    fill?: GridFill;
    gap?: number | null;
}
export declare class GridLayout extends BuiltinLayout {
    static readonly KIND = 2;
    static readonly SHAPES: Record<string, [number, string, boolean]>;
    static readonly ACTION_SHAPES: Record<string, [number, string, boolean]>;
    constructor(options?: GridLayoutOptions);
    get columns(): number;
    set columns(value: number);
    get fill(): GridFill;
    set fill(value: GridFill);
    get gap(): number | null;
    set gap(value: number | null);
}
export interface MonocleLayoutOptions {
}
export declare class MonocleLayout extends BuiltinLayout {
    static readonly KIND = 3;
    static readonly SHAPES: Record<string, [number, string, boolean]>;
    static readonly ACTION_SHAPES: Record<string, [number, string, boolean]>;
    constructor(options?: MonocleLayoutOptions);
}
export declare const DwindleSplit: {
    readonly ASPECT: 0;
    readonly HORIZONTAL: 1;
    readonly VERTICAL: 2;
};
export type DwindleSplit = (typeof DwindleSplit)[keyof typeof DwindleSplit];
export declare const DwindleForceSplit: {
    readonly BY_POINTER: 0;
    readonly FIRST: 1;
    readonly SECOND: 2;
};
export type DwindleForceSplit = (typeof DwindleForceSplit)[keyof typeof DwindleForceSplit];
export declare const DwindleSplitBias: {
    readonly FIRST: 0;
    readonly OPENED: 1;
};
export type DwindleSplitBias = (typeof DwindleSplitBias)[keyof typeof DwindleSplitBias];
export declare const DwindlePointer: {
    readonly NONE: 0;
    readonly SEAT: 1;
    readonly FIXED: 2;
};
export type DwindlePointer = (typeof DwindlePointer)[keyof typeof DwindlePointer];
export declare const DwindleDirection: {
    readonly LEFT: 0;
    readonly RIGHT: 1;
    readonly UP: 2;
    readonly DOWN: 3;
    readonly NONE: 4;
};
export type DwindleDirection = (typeof DwindleDirection)[keyof typeof DwindleDirection];
export declare const DwindlePreselected: {
    readonly LEFT: 0;
    readonly RIGHT: 1;
    readonly UP: 2;
    readonly DOWN: 3;
    readonly NONE: 4;
};
export type DwindlePreselected = (typeof DwindlePreselected)[keyof typeof DwindlePreselected];
export interface DwindleLayoutOptions {
    split?: DwindleSplit;
    ratio?: number;
    gap?: number | null;
    forceSplit?: DwindleForceSplit;
    preserveSplit?: boolean;
    smartSplit?: boolean;
    smartResizing?: boolean;
    permanentDirectionOverride?: boolean;
    splitWidthMultiplier?: number;
    useActiveForSplits?: boolean;
    splitBias?: DwindleSplitBias;
    pointer?: DwindlePointer;
    pointerX?: number | null;
    pointerY?: number | null;
}
export declare class DwindleLayout extends BuiltinLayout {
    static readonly KIND = 4;
    static readonly SHAPES: Record<string, [number, string, boolean]>;
    static readonly ACTION_SHAPES: Record<string, [number, string, boolean]>;
    constructor(options?: DwindleLayoutOptions);
    get split(): DwindleSplit;
    set split(value: DwindleSplit);
    get ratio(): number;
    set ratio(value: number);
    get gap(): number | null;
    set gap(value: number | null);
    get forceSplit(): DwindleForceSplit;
    set forceSplit(value: DwindleForceSplit);
    get preserveSplit(): boolean;
    set preserveSplit(value: boolean);
    get smartSplit(): boolean;
    set smartSplit(value: boolean);
    get smartResizing(): boolean;
    set smartResizing(value: boolean);
    get permanentDirectionOverride(): boolean;
    set permanentDirectionOverride(value: boolean);
    get splitWidthMultiplier(): number;
    set splitWidthMultiplier(value: number);
    get useActiveForSplits(): boolean;
    set useActiveForSplits(value: boolean);
    get splitBias(): DwindleSplitBias;
    set splitBias(value: DwindleSplitBias);
    get pointer(): DwindlePointer;
    set pointer(value: DwindlePointer);
    get pointerX(): number | null;
    set pointerX(value: number | null);
    get pointerY(): number | null;
    set pointerY(value: number | null);
    toggleSplit(): boolean;
    swapSplit(): boolean;
    rotateSplit(angle: number): boolean;
    moveToRoot(stable: boolean): boolean;
    splitRatio(amount: number): boolean;
    setSplitRatio(ratio: number): boolean;
    preselect(direction: DwindleDirection): void;
    preselected(): DwindlePreselected;
    pseudotile(width?: number | null, height?: number | null): boolean;
    isPseudotiled(client: number): boolean;
}
export interface StripLayoutOptions {
    column?: number;
}
export declare class StripLayout extends BuiltinLayout {
    static readonly KIND = 5;
    static readonly SHAPES: Record<string, [number, string, boolean]>;
    static readonly ACTION_SHAPES: Record<string, [number, string, boolean]>;
    constructor(options?: StripLayoutOptions);
    get column(): number;
    set column(value: number);
    scrollTo(offset?: number | null): void;
    focusLeft(): void;
    focusRight(): void;
    pin(): number | null;
}
export interface ArcLayoutOptions {
    radius?: number;
    spacing?: number;
}
export declare class ArcLayout extends BuiltinLayout {
    static readonly KIND = 6;
    static readonly SHAPES: Record<string, [number, string, boolean]>;
    static readonly ACTION_SHAPES: Record<string, [number, string, boolean]>;
    constructor(options?: ArcLayoutOptions);
    get radius(): number;
    set radius(value: number);
    get spacing(): number;
    set spacing(value: number);
}
export interface CarouselLayoutOptions {
    fov?: number;
    cardWidth?: number;
    cardHeight?: number;
}
export declare class CarouselLayout extends BuiltinLayout {
    static readonly KIND = 7;
    static readonly SHAPES: Record<string, [number, string, boolean]>;
    static readonly ACTION_SHAPES: Record<string, [number, string, boolean]>;
    constructor(options?: CarouselLayoutOptions);
    get fov(): number;
    set fov(value: number);
    get cardWidth(): number;
    set cardWidth(value: number);
    get cardHeight(): number;
    set cardHeight(value: number);
    rotate(steps: number): void;
}
export interface MoveBindingOptions {
    trigger?: string;
    priority?: number;
    snap?: boolean;
    snapMargin?: number;
}
export declare class MoveBinding extends BuiltinBinding {
    static readonly KIND = 1;
    static readonly SHAPES: Record<string, [number, string, boolean]>;
    constructor(options?: MoveBindingOptions);
}
export declare const ResizeGrip: {
    readonly QUADRANT: 0;
    readonly NEAREST: 1;
    readonly TOP_LEFT: 2;
    readonly TOP_RIGHT: 3;
    readonly BOTTOM_LEFT: 4;
    readonly BOTTOM_RIGHT: 5;
};
export type ResizeGrip = (typeof ResizeGrip)[keyof typeof ResizeGrip];
export interface ResizeBindingOptions {
    trigger?: string;
    priority?: number;
    grip?: ResizeGrip;
}
export declare class ResizeBinding extends BuiltinBinding {
    static readonly KIND = 2;
    static readonly SHAPES: Record<string, [number, string, boolean]>;
    constructor(options?: ResizeBindingOptions);
}
export interface SwapBindingOptions {
    trigger?: string;
    priority?: number;
}
export declare class SwapBinding extends BuiltinBinding {
    static readonly KIND = 3;
    static readonly SHAPES: Record<string, [number, string, boolean]>;
    constructor(options?: SwapBindingOptions);
}
export interface SplitResizeBindingOptions {
    trigger?: string;
    priority?: number;
}
export declare class SplitResizeBinding extends BuiltinBinding {
    static readonly KIND = 4;
    static readonly SHAPES: Record<string, [number, string, boolean]>;
    constructor(options?: SplitResizeBindingOptions);
}
export {};
