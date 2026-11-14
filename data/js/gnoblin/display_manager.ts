// What displays and profiles do to the shell: `DisplayManager` reconciles the `OutputProfile`s and `Display`s a script
// declared with the connected outputs. It applies a profile when its monitor is connected (and again when the monitor
// returns), gives every output one display (the shell makes a `single` one where no script display presents it), asks the
// backend to mirror or span the outputs of a display, and shows a viewport's workspace on the display's desk.
//
// The host module owns the collections, the events and the desks; it gives this module a `DisplayBackend` and calls
// `frame()` each layout pass. Nothing here touches the compositor directly.

import { Camera3D } from "gnoblin/scene";
import { Camera2D, Display, OutputProfile, implicitDisplay, isIdentity } from "gnoblin/display";
import type { OutputAlign, OutputSetting, OutputView, PlacementTarget } from "gnoblin/display";
import {
    displayState,
    profileMatches,
    profileSetting,
    profileState,
    relativeTarget,
    viewportState,
} from "gnoblin/display";

/** A `Collection<Output>` the manager keeps equal to the outputs a display presents. */
export interface OutputSet {
    readonly collection: object;
    set(outputs: readonly OutputView[]): void;
}

/** What the manager needs of the shell: outputs, desks, the output requests and the events of the collections. */
export interface DisplayBackend {
    /** The connected outputs, in the order the compositor made them. */
    outputs(): readonly OutputView[];
    /** The output flagged primary, else the first. */
    primary(): OutputView | null;
    /** Whether the output has a desk of its own: it is on and not a mirror. */
    hasDesk(output: OutputView): boolean;
    /** The workspace the output shows (a spanned output shows the workspace its span does), or null. */
    shownOn(output: OutputView): object | null;
    /** Whether the workspace is in `gnoblin.workspaces`. */
    attached(workspace: object): boolean;
    /** Shows `workspace` on `output` as `gnoblin.workspaces.show` does; throws as it does. */
    show(workspace: object, output: OutputView): void;
    /** Makes `lead` show one workspace over the union of `members`; null ends the span. */
    span(lead: OutputView, members: readonly OutputView[] | null): void;
    /** The output `output` mirrors, or null. */
    mirrorOf(output: OutputView): OutputView | null;
    /** Mirrors `source` on `output`, or stops mirroring with null. */
    mirror(output: OutputView, source: OutputView | null, path: string): void;
    /** Sends one output request; a refusal is reported under `path`. */
    apply(output: OutputView, setting: OutputSetting, path: string): void;
    /** Whether a request for `output` has not been answered yet. */
    pending(output: OutputView): boolean;
    /** Reports `text` once per generation. */
    say(text: string): void;
    error(code: string, target: string, message: string): Error;
    /** A new collection of outputs for one display. */
    outputSet(): OutputSet;
    /** The collections' events: the member joined or left. */
    listed(member: Display | OutputProfile): void;
    unlisted(member: Display | OutputProfile): void;
}

/** What was last sent to one output for one profile. */
interface Sent {
    base: string | null;
    place: string | null;
}

const CAMERA3D_NOTICE = "not applied: camera3d (needs a projected workspace)";
const CAMERA2D_NOTICE =
    "not applied: camera2d zoom, position and rotation (windows sit under the tree root: there is no per-display root to transform)";

/** The name a message gives an output. */
function nameOf(output: OutputView): string {
    return output.connector === "" ? output.name : output.connector;
}

/** The top-left corner that puts an output of `size` against `target` on `side`, aligned as asked. */
function placed(
    side: "rightOf" | "leftOf" | "above" | "below",
    align: OutputAlign | undefined,
    target: OutputView,
    size: { width: number; height: number },
): { x: number; y: number } {
    const horizontal = side === "rightOf" || side === "leftOf";
    // Along the shared edge: the start by default, as the specification's `align: "top"` example reads.
    const along = (own: number, theirs: number, at: OutputAlign | undefined): number => {
        if (at === "center") return Math.round((theirs - own) / 2);
        if (at === "bottom" || at === "right") return theirs - own;
        return 0;
    };
    if (horizontal) {
        return {
            x: side === "rightOf" ? target.x + target.width : target.x - size.width,
            y: target.y + along(size.height, target.height, align),
        };
    }
    return {
        x: target.x + along(size.width, target.width, align),
        y: side === "below" ? target.y + target.height : target.y - size.height,
    };
}

/** Reconciles profiles and displays with the connected outputs. */
export class DisplayManager {
    readonly #backend: DisplayBackend;
    readonly #profiles: OutputProfile[];
    readonly #displays: Display[];
    #dirty = true;
    /** The display that presents each output, rebuilt by `reflow`. */
    #owners = new Map<OutputView, Display>();
    /** The display the shell made for each output it made one for. */
    readonly #implicit = new Map<OutputView, Display>();
    readonly #sent = new WeakMap<OutputView, Map<OutputProfile, Sent>>();

    /** `profiles` and `displays` are the lists behind `gnoblin.outputs.profiles` and `gnoblin.displays`. */
    constructor(backend: DisplayBackend, profiles: OutputProfile[], displays: Display[]) {
        this.#backend = backend;
        this.#profiles = profiles;
        this.#displays = displays;
    }

    /** Something changed that `reflow` must look at. */
    changed(): void {
        this.#dirty = true;
    }

    // ---- Profiles ----

    /** @throws GnoblinError `duplicate` when a profile with the same id is in the collection */
    addProfile(profile: OutputProfile): void {
        if (this.#profiles.includes(profile)) {
            return;
        }
        const { id } = profileState(profile);
        if (this.#profiles.some((other) => profileState(other).id === id)) {
            throw this.#backend.error(
                "duplicate",
                "gnoblin.outputs.profiles.add",
                `there is already a profile "${id}"; give the profile an id of its own`,
            );
        }
        this.#profiles.push(profile);
        this.#backend.listed(profile);
        this.#dirty = true;
        this.reflow();
    }

    /** Stops managing the profile. The compositor keeps the arrangement it applied last. */
    removeProfile(profile: OutputProfile): boolean {
        const at = this.#profiles.indexOf(profile);
        if (at < 0) {
            return false;
        }
        this.#profiles.splice(at, 1);
        this.#backend.unlisted(profile);
        return true;
    }

    // ---- Displays ----

    /**
     * Adds a display the script made.
     *
     * @throws GnoblinError `output-conflict` naming both displays when one of its outputs is presented by another, or
     *   `duplicate` when a display has its id
     */
    addDisplay(display: Display): void {
        if (this.#displays.includes(display)) {
            return;
        }
        this.#checkAdd(display, new Set(), "gnoblin.displays.add");
        this.#insert(display, this.#displays.length);
        this.reflow();
    }

    /**
     * Removes a script display: its mirror and span end and each of its outputs gets a display of the shell's.
     * Returns false for a display of the shell's own, which stays, and for one that is not a member.
     */
    removeDisplay(display: Display): boolean {
        if (!this.#displays.includes(display) || displayState(display).implicit) {
            return false;
        }
        this.#drop(display);
        this.#dirty = true;
        this.reflow();
        return true;
    }

    /**
     * Swaps `old` for `next` as one validated change: `old` and every other script display that shares an output with
     * `next` leave, then `next` takes `old`'s place. Nothing changes when the check fails.
     *
     * @throws GnoblinError `not-a-member` when `old` is not in the collection, `duplicate` when `next` is or has an id
     *   that stays
     */
    replaceDisplay(old: Display, next: Display): void {
        if (!this.#displays.includes(old) || displayState(old).implicit) {
            throw this.#backend.error(
                "not-a-member",
                "gnoblin.displays.replace",
                `${old} is not a display of the script`,
            );
        }
        if (this.#displays.includes(next)) {
            throw this.#backend.error("duplicate", "gnoblin.displays.replace", `${next} is in the collection already`);
        }
        const leaving = new Set<Display>([old]);
        for (const other of this.#displays) {
            if (!displayState(other).implicit && this.#overlap(next, other) !== null) {
                leaving.add(other);
            }
        }
        this.#checkAdd(next, leaving, "gnoblin.displays.replace");
        const at = this.#displays.indexOf(old);
        const before = this.#displays.slice(0, at).filter((display) => !leaving.has(display)).length;
        for (const display of leaving) {
            this.#drop(display);
        }
        this.#insert(next, Math.min(before, this.#displays.length));
        this.#dirty = true;
        this.reflow();
    }

    /** Removes every display of the script. */
    clearDisplays(): void {
        for (const display of [...this.#displays]) {
            if (!displayState(display).implicit) {
                this.#drop(display);
            }
        }
        this.#dirty = true;
        this.reflow();
    }

    /** The display under `point` in the compositor's space, else the primary output's; null when no display has an output. */
    displayAt(point: { x: number; y: number }): Display | null {
        if (this.#dirty) {
            this.reflow();
        }
        for (const output of this.#backend.outputs()) {
            const inside =
                point.x >= output.x &&
                point.x < output.x + output.width &&
                point.y >= output.y &&
                point.y < output.y + output.height;
            const owner = inside ? this.#owners.get(output) : undefined;
            if (owner !== undefined) {
                return owner;
            }
        }
        const primary = this.#backend.primary();
        return primary === null ? null : (this.#owners.get(primary) ?? null);
    }

    /** What the display's viewport shows: the workspace on its output, undefined while it has no output with a desk. */
    shown(display: Display): object | null | undefined {
        const lead = displayState(display).live[0];
        if (lead === undefined || !lead.connected || !this.#backend.hasDesk(lead)) {
            return undefined;
        }
        return this.#backend.shownOn(lead);
    }

    /**
     * Shows `workspace` on the display, or keeps it for when the display has an output.
     *
     * @throws GnoblinError as `gnoblin.workspaces.show` does, when the workspace is not in `gnoblin.workspaces`
     */
    assignContent(display: Display, workspace: object): void {
        const state = displayState(display);
        const lead = state.live[0];
        if (lead === undefined || !lead.connected || !this.#backend.hasDesk(lead)) {
            state.wanted = workspace;
            return;
        }
        this.#backend.show(workspace, lead);
        state.wanted = null;
    }

    /** The `Collection<Output>` of the outputs the display presents. */
    outputsOf(display: Display): object {
        const state = displayState(display);
        if (state.outputs === null) {
            state.outputs = this.#backend.outputSet();
            state.outputs.set(state.live);
        }
        return state.outputs.collection;
    }

    /**
     * What a transition callback can change that is not a tree write, to put back when it fails: the displays' wanted
     * workspaces, cameras and colours.
     */
    snapshot(): () => void {
        const saved = this.#displays.map((display) => {
            const state = displayState(display);
            const view = viewportState(state.viewport);
            return {
                state,
                wanted: state.wanted,
                view,
                content: view.content,
                camera: view.camera,
                color: view.clearColor,
            };
        });
        return () => {
            for (const item of saved) {
                item.state.wanted = item.wanted;
                Object.assign(item.view, { content: item.content, camera: item.camera, clearColor: item.color });
            }
            this.#dirty = true;
        };
    }

    /** Called each layout pass: reconciles when something changed, then reports what the cameras ask of the renderer. */
    frame(): void {
        if (this.#dirty) {
            this.reflow();
        }
        for (const display of this.#displays) {
            const view = viewportState(displayState(display).viewport);
            const camera = view.camera;
            if (camera instanceof Camera3D) {
                this.#backend.say(CAMERA3D_NOTICE);
            } else if (camera instanceof Camera2D && !isIdentity(camera)) {
                this.#backend.say(CAMERA2D_NOTICE);
            }
            if (view.clearColor !== "#000000") {
                this.#backend.say(
                    "not applied: viewport.clearColor (the compositor has no background colour behind a workspace)",
                );
            }
        }
    }

    /** Applies the profiles, settles who presents which output, and enacts each display. */
    reflow(): void {
        this.#dirty = false;
        const live = this.#backend.outputs();
        this.#applyProfiles(live);
        const owners = new Map<OutputView, Display>();
        for (const display of this.#displays) {
            const state = displayState(display);
            if (state.implicit) {
                continue;
            }
            state.live = this.#resolve(display, live).filter((output) => {
                const held = owners.get(output);
                if (held !== undefined) {
                    this.#backend.say(
                        `not applied: display "${state.id}" presents ${nameOf(output)}, which display "${displayState(held).id}" already does`,
                    );
                    return false;
                }
                owners.set(output, display);
                return true;
            });
        }
        this.#syncImplicit(live, owners);
        this.#owners = owners;
        for (const display of [...this.#displays]) {
            this.#enact(display);
        }
    }

    // ---- Checks ----

    /** What `a` and `b` both present, as a phrase, or null when they share nothing. */
    #overlap(a: Display, b: Display): string | null {
        const left = displayState(a);
        const right = displayState(b);
        for (const selector of left.selectors) {
            if (right.selectors.includes(selector)) {
                return selector instanceof OutputProfile
                    ? `profile "${profileState(selector).id}"`
                    : `output ${nameOf(selector)}`;
            }
        }
        const live = this.#backend.outputs();
        const theirs = this.#resolve(b, live);
        const shared = this.#resolve(a, live).find((output) => theirs.includes(output));
        return shared === undefined ? null : `output ${nameOf(shared)}`;
    }

    /** Throws when `display` clashes with a script display that is not in `leaving`. */
    #checkAdd(display: Display, leaving: ReadonlySet<Display>, site: string): void {
        const state = displayState(display);
        for (const other of this.#displays) {
            const held = displayState(other);
            if (held.implicit || leaving.has(other)) {
                continue;
            }
            if (held.id === state.id) {
                throw this.#backend.error("duplicate", site, `there is already a display "${state.id}"`);
            }
            const shared = this.#overlap(display, other);
            if (shared !== null) {
                throw this.#backend.error(
                    "output-conflict",
                    site,
                    `display "${state.id}" cannot present ${shared}: display "${held.id}" does already ` +
                        `(use gnoblin.displays.replace("${held.id}", ...) to swap them)`,
                );
            }
        }
    }

    /** The connected outputs `display` presents, in selector order. A profile gives the first match not used yet. */
    #resolve(display: Display, live: readonly OutputView[]): OutputView[] {
        const found: OutputView[] = [];
        for (const selector of displayState(display).selectors) {
            const output =
                selector instanceof OutputProfile
                    ? live.find((one) => !found.includes(one) && profileMatches(selector, one))
                    : live.includes(selector) && selector.connected && !found.includes(selector)
                      ? selector
                      : undefined;
            if (output !== undefined) {
                found.push(output);
            }
        }
        return found;
    }

    // ---- Membership ----

    #insert(display: Display, at: number): void {
        // The shell's own display of the same name or for the same output gives way: it is what the script replaces.
        for (const other of [...this.#displays]) {
            const held = displayState(other);
            if (held.implicit && held.id === displayState(display).id) {
                this.#drop(other);
            }
        }
        this.#displays.splice(Math.min(at, this.#displays.length), 0, display);
        this.#backend.listed(display);
    }

    /** Takes a display out: its mirror and span end, and it presents nothing. */
    #drop(display: Display): void {
        this.#release(display);
        const at = this.#displays.indexOf(display);
        if (at >= 0) {
            this.#displays.splice(at, 1);
        }
        const state = displayState(display);
        state.live = [];
        state.outputs?.set([]);
        for (const [output, held] of this.#implicit) {
            if (held === display) {
                this.#implicit.delete(output);
            }
        }
        this.#backend.unlisted(display);
    }

    /** Gives each output with a desk that no script display presents a display of the shell's, and retires the rest. */
    #syncImplicit(live: readonly OutputView[], owners: Map<OutputView, Display>): void {
        for (const [output, display] of [...this.#implicit]) {
            if (!live.includes(output) || !this.#backend.hasDesk(output) || owners.has(output)) {
                this.#drop(display);
            }
        }
        for (const output of live) {
            if (owners.has(output) || !this.#backend.hasDesk(output)) {
                continue;
            }
            let display = this.#implicit.get(output);
            if (display === undefined) {
                display = implicitDisplay(output, `display-${nameOf(output)}`);
                this.#implicit.set(output, display);
                this.#displays.push(display);
                this.#backend.listed(display);
            }
            displayState(display).live = [output];
            owners.set(output, display);
        }
    }

    // ---- Enacting a display ----

    /** Undoes what a display did to its outputs: the mirrors it made and the span it holds. */
    #release(display: Display): void {
        this.#unmirror(display);
        this.#endSpan(display);
    }

    /** Brings the hardware and the desks to what `display` says. */
    #enact(display: Display): void {
        const state = displayState(display);
        state.outputs?.set(state.live);
        const [lead, ...rest] = state.live;
        if (lead === undefined) {
            return;
        }
        const members = state.mode === "single" ? [] : rest;
        if (state.mode === "mirror" && members.length > 0) {
            this.#mirror(display, lead, members);
        } else {
            this.#unmirror(display);
        }
        if (state.mode === "span" && members.length > 0) {
            if (state.spanLead !== lead) {
                this.#endSpan(display);
            }
            this.#backend.span(lead, [lead, ...members]);
            state.spanLead = lead;
        } else {
            this.#endSpan(display);
        }
        this.#settleContent(display, lead);
    }

    #unmirror(display: Display): void {
        const state = displayState(display);
        for (const [output, source] of state.mirrored) {
            if (output.connected && this.#backend.mirrorOf(output) === source) {
                this.#backend.mirror(output, null, `displays.${state.id}`);
            }
        }
        state.mirrored.clear();
    }

    #endSpan(display: Display): void {
        const state = displayState(display);
        if (state.spanLead !== null && state.spanLead.connected) {
            this.#backend.span(state.spanLead, null);
        }
        state.spanLead = null;
    }

    #mirror(display: Display, lead: OutputView, members: readonly OutputView[]): void {
        const state = displayState(display);
        for (const output of members) {
            state.mirrored.set(output, lead);
            if (this.#backend.mirrorOf(output) !== lead && !this.#backend.pending(output)) {
                this.#backend.mirror(output, lead, `displays.${state.id}`);
            }
        }
    }

    /** Shows the workspace a viewport waits to show, and remembers what the desk shows for when the display is parked. */
    #settleContent(display: Display, lead: OutputView): void {
        const state = displayState(display);
        if (!this.#backend.hasDesk(lead)) {
            return;
        }
        const wanted = state.wanted;
        if (wanted !== null) {
            if (this.#backend.attached(wanted)) {
                this.#backend.show(wanted, lead);
                state.wanted = null;
            } else {
                this.#backend.say(
                    `not applied: display "${state.id}" shows a workspace that is not in gnoblin.workspaces (add it first)`,
                );
            }
        }
        const shown = this.#backend.shownOn(lead);
        if (shown !== null) {
            viewportState(state.viewport).content = shown;
        }
    }

    // ---- Profiles on hardware ----

    #sentFor(output: OutputView, profile: OutputProfile): Sent {
        let by = this.#sent.get(output);
        if (by === undefined) {
            by = new Map();
            this.#sent.set(output, by);
        }
        let sent = by.get(profile);
        if (sent === undefined) {
            sent = { base: null, place: null };
            by.set(profile, sent);
        }
        return sent;
    }

    /** The first connected output `profile` is for, or null while its monitor is not connected. */
    #hardware(profile: OutputProfile, live: readonly OutputView[]): OutputView | null {
        return live.find((output) => profileMatches(profile, output)) ?? null;
    }

    #applyProfiles(live: readonly OutputView[]): void {
        // A neighbour first: its geometry is what a relative position reads.
        const depth = (profile: OutputProfile): number => {
            const target = relativeTarget(profileState(profile).position)?.target;
            return target instanceof OutputProfile ? 1 + depth(target) : 0;
        };
        const ordered = [...this.#profiles].sort((a, b) => depth(a) - depth(b));
        for (const profile of ordered) {
            const output = this.#hardware(profile, live);
            if (output !== null) {
                this.#applyProfile(profile, output, live);
            }
        }
    }

    /** One step of applying a profile: its own settings first, then, once those are answered, its relative position. */
    #applyProfile(profile: OutputProfile, output: OutputView, live: readonly OutputView[]): void {
        const state = profileState(profile);
        const path = `outputs.profiles.${state.id}`;
        const sent = this.#sentFor(output, profile);
        const setting = profileSetting(profile);
        const text = JSON.stringify(setting);
        if (state.vrr === true) {
            this.#backend.say(`not applied: OutputProfile "${state.id}" vrr (the output request has no vrr key yet)`);
        }
        if (text !== "{}" && sent.base !== text) {
            if (!this.#backend.pending(output)) {
                sent.base = text;
                this.#backend.apply(output, setting, path);
            }
            return;
        }
        const relative = relativeTarget(state.position);
        if (relative === null || this.#backend.pending(output)) {
            return;
        }
        const neighbour = this.#neighbour(relative.target, live);
        if (neighbour === null) {
            this.#backend.say(
                `not applied: OutputProfile "${state.id}" position (the output it is placed against is not connected)`,
            );
            return;
        }
        if (this.#backend.pending(neighbour)) {
            return;
        }
        const at = placed(relative.side, relative.align, neighbour, output);
        const place = `${at.x},${at.y}`;
        if (sent.place === place) {
            return;
        }
        sent.place = place;
        if (output.x !== at.x || output.y !== at.y) {
            this.#backend.apply(output, { position: at }, path);
        }
    }

    #neighbour(target: PlacementTarget, live: readonly OutputView[]): OutputView | null {
        if (target instanceof OutputProfile) {
            return this.#hardware(target, live);
        }
        return target.connected ? target : null;
    }
}
