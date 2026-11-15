// `XRRuntime`: the drag loop and the message handling of the XR seat. It owns the controllers, the room and the
// drags a controller binding runs, over the world the host hands it.

import type { Vector3 } from "gnoblin/scene/vector";
import type { View } from "gnoblin/scene/view";
import { XRList } from "gnoblin/xr/controller";
import type { XRController } from "gnoblin/xr/controller";
import { matches } from "gnoblin/xr/extensions";
import { SpatialLayout } from "gnoblin/xr/layout";
import { XRRoom } from "gnoblin/xr/room";
import { XRExtension, XRSession } from "gnoblin/xr/session";
import type { XROptions } from "gnoblin/xr/session";
import type { NodeNumber, XRWorld } from "gnoblin/xr/types";
import { NEVER, NOW, onWake } from "gnoblin/wake";
import { pickView, XRViewport } from "gnoblin/xr/viewport";
import { XRControllerDragBinding } from "gnoblin/xr_bindings";
import type { XRDragEvent } from "gnoblin/xr_bindings";

interface Run {
    readonly controller: XRController;
    readonly binding: XRControllerDragBinding;
    readonly state: object;
    readonly view: View;
    readonly hit: Vector3;
    last: number;
    /** Whether the panel was pinned before the drag, so a cancel can give it back to the ring. */
    readonly wasPinned: boolean;
}

/**
 * The shell's side of XR, one per shell. It follows the seat: while the program has a session it builds the room, keeps
 * the panels current and runs the drags; when the session ends it takes all of it down.
 */
export class XRRuntime {
    readonly #world: XRWorld;
    readonly #sessions: XRList<XRSession>;
    readonly #listeners: object[] = [];
    readonly #runs = new Map<XRController, Run>();
    readonly #declined = new Set<XRController>();
    readonly #since = new Map<XRController, { key: string; at: number }>();
    #extension: XRExtension | null = null;
    #session: XRSession | null = null;
    #stock: XRRoom | null = null;
    options: XROptions = {};

    constructor(world: XRWorld) {
        this.#world = world;
        this.#sessions = new XRList<XRSession>(world.report);
        // A running session follows the room and the drags every frame.
        onWake(() => (this.#session === null ? NEVER : NOW));
    }

    get extension(): XRExtension | null {
        return this.#extension;
    }

    addListener(listener: object): void {
        if (typeof listener !== "object" || listener === null) {
            throw new TypeError("addListener: the listener must be an object");
        }
        this.#listeners.push(listener);
    }

    removeListener(listener: object): void {
        const at = this.#listeners.indexOf(listener);
        if (at >= 0) this.#listeners.splice(at, 1);
    }

    #tell(method: "onSessionStarted" | "onSessionEnded", session: XRSession): void {
        for (const listener of [...this.#listeners]) {
            const handler = Reflect.get(listener, method);
            if (typeof handler === "function") {
                try {
                    handler.call(listener, session);
                } catch (error) {
                    this.#world.report(`XRListener.${method}`, error);
                }
            }
        }
    }

    /** Whether `gnoblin/extensions/xr` may be imported now: the session declared XR, or the program has a session. */
    available(): boolean {
        return (
            this.#world.declared() || this.#extension !== null || this.#world.seats().some((seat) => seat.name === "xr")
        );
    }

    #adopt(viewport: XRViewport): void {
        if (this.#stock !== null && viewport.content !== this.#stock) {
            // A script's own room replaces the stock one.
            this.#dropStock();
        }
        viewport.apply();
        if (this.#extension !== null && viewport.content instanceof XRRoom) {
            this.#extension.room = viewport.content;
        }
    }

    #dropStock(): void {
        const stock = this.#stock;
        this.#stock = null;
        if (stock !== null) {
            if (this.#extension?.room === stock) this.#extension.room = null;
            if (stock.alive) stock.destroy();
        }
    }

    /** Builds the stock room unless the script said not to or a room is in the tree already (the user's). */
    #buildStock(session: XRSession): void {
        if (this.options.room === false || this.#stock !== null) {
            return;
        }
        const taken = this.#world.nodes("Node").some((node) => this.#world.scene.get(node, "name") === "room");
        if (taken) {
            return;
        }
        const room = new XRRoom({ layout: new SpatialLayout(this.options.layout), accept: this.options.accept });
        this.#stock = room;
        const viewport = new XRViewport({ content: room, camera: room.head, composition: "auto" });
        viewport.apply();
        session.viewport = viewport;
    }

    #start(seat: NodeNumber): void {
        const session = new XRSession(this.#world, seat, (viewport) => this.#adopt(viewport));
        this.#session = session;
        this.#sessions.add(session);
        this.#extension = new XRExtension(this.#sessions);
        this.#tell("onSessionStarted", session);
        this.#buildStock(session);
    }

    #end(): void {
        const session = this.#session;
        if (session === null) {
            return;
        }
        this.#cancelRuns();
        session.end();
        this.#session = null;
        this.#dropStock();
        for (const controller of [...session.controllers]) session.controllers.remove(controller);
        this.#sessions.remove(session);
        this.#extension = null;
        this.#since.clear();
        this.#declined.clear();
        this.#tell("onSessionEnded", session);
    }

    /** Runs once a frame: follows the seat, then keeps the room and the drags current. */
    frame(): void {
        const seat = this.#world.seats().find((candidate) => candidate.name === "xr");
        if (seat === undefined) {
            this.#end();
            return;
        }
        if (this.#session !== null && this.#session.id !== String(seat.node)) {
            this.#end();
        }
        if (this.#session === null) {
            this.#start(seat.node);
        }
        const session = this.#session;
        if (session === null) {
            return;
        }
        session.syncControllers();
        const room = this.#extension?.room ?? null;
        if (room !== null && room.alive) {
            try {
                room.reconcile(
                    this.#world.clients(),
                    this.#world.focusedClient(),
                    (client) => this.#world.parentKind(client) === "window",
                );
            } catch (error) {
                this.#world.report("XRRoom.reconcile", error);
            }
            this.#drags(session, room);
        } else {
            this.#cancelRuns();
        }
    }

    // -- Drags ---------------------------------------------------------------------------------------------------

    #event(
        session: XRSession,
        room: XRRoom,
        run: Pick<Run, "controller" | "binding" | "view" | "hit">,
        now: number,
        last: number,
    ): XRDragEvent {
        return {
            timestamp: now,
            deltaTime: Math.min(Math.max(now - last, 0) / 1000, 0.1),
            context: session.input,
            device: null,
            modifiers: new Set<string>(),
            controller: run.controller,
            view: run.view,
            ray: run.controller.ray,
            hit: run.hit,
            trigger: run.binding.trigger,
            hand: run.controller.hand,
            room,
            unit: room.unit,
        };
    }

    #hook(site: string, call: () => void): boolean {
        try {
            call();
            return true;
        } catch (error) {
            this.#world.report(site, error);
            return false;
        }
    }

    #finish(run: Run, hook: "onEnd" | "onCancel", session: XRSession, room: XRRoom, now: number): void {
        this.#runs.delete(run.controller);
        this.#unclaim(run);
        this.#hook(`${run.binding.id}.${hook}`, () =>
            run.binding[hook](this.#event(session, room, run, now, run.last), run.state),
        );
        // A finished drag leaves the panel where it was put (pinned); a cancelled one gives it back to the ring.
        if (hook === "onCancel" && !run.wasPinned) room.unpin(run.view);
    }

    /** Drops what a drag claimed. A controller that is gone holds no claim, so only a refused write is reported. */
    #unclaim(run: Pick<Run, "controller" | "binding">): void {
        this.#hook(`${run.binding.id}.release`, () => run.controller.release(run.binding));
    }

    #cancelRuns(): void {
        const session = this.#session;
        const room = this.#extension?.room ?? null;
        for (const run of [...this.#runs.values()]) {
            this.#runs.delete(run.controller);
            this.#unclaim(run);
            if (session !== null && room !== null) {
                this.#hook(`${run.binding.id}.onCancel`, () =>
                    run.binding.onCancel(this.#event(session, room, run, this.#world.now(), run.last), run.state),
                );
            }
            if (room !== null && !run.wasPinned) room.unpin(run.view);
        }
    }

    /** Cancels every drag in flight (once each) and ends the session's hold. The shell calls it when a generation ends. */
    release(): void {
        this.#cancelRuns();
    }

    #drags(session: XRSession, room: XRRoom): void {
        const layout = room.layout;
        const now = this.#world.now();
        const bindings =
            layout === null
                ? []
                : [...layout.bindings].filter(
                      (b): b is XRControllerDragBinding => b instanceof XRControllerDragBinding,
                  );
        for (const controller of session.controllers) {
            const key = [...controller.buttons].sort().join("+");
            const seen = this.#since.get(controller);
            if (seen === undefined || seen.key !== key) {
                this.#since.set(controller, { key, at: now });
            }
            const run = this.#runs.get(controller);
            if (run !== undefined) {
                const chord = run.binding.parsed.buttons;
                const held =
                    chord.length === controller.buttons.size && chord.every((button) => controller.buttons.has(button));
                const still = bindings.includes(run.binding) && run.binding.enabled;
                if (!controller.alive || !controller.tracked || !still || !run.view.alive || run.view.orphaned) {
                    this.#finish(run, "onCancel", session, room, now);
                } else if (!held) {
                    this.#finish(run, "onEnd", session, room, now);
                } else {
                    const event = this.#event(session, room, run, now, run.last);
                    run.last = now;
                    if (!this.#hook(`${run.binding.id}.onUpdate`, () => run.binding.onUpdate(event, run.state))) {
                        this.#finish(run, "onCancel", session, room, now);
                    }
                }
                continue;
            }
            if (controller.buttons.size === 0) {
                this.#declined.delete(controller);
                continue;
            }
            const settled = now - (this.#since.get(controller)?.at ?? now) >= session.chordSettleMs;
            if (this.#declined.has(controller) || !controller.tracked || !settled) {
                continue;
            }
            const candidates = bindings
                .filter((binding) => binding.enabled && matches(binding.parsed, controller))
                .sort((a, b) => b.priority - a.priority);
            if (candidates.length === 0) {
                continue;
            }
            const ray = controller.ray;
            const hit = pickView(room, ray);
            for (const binding of candidates) {
                const event: XRDragEvent = {
                    ...this.#event(
                        session,
                        room,
                        { controller, binding, view: hit?.view as View, hit: hit?.point as Vector3 },
                        now,
                        now,
                    ),
                    view: hit?.view,
                    hit: hit?.point,
                };
                let state: object | false = false;
                if (!this.#hook(`${binding.id}.onStart`, () => (state = binding.onStart(event)))) {
                    continue;
                }
                if (state !== false && typeof state === "object" && state !== null && hit !== null) {
                    const wasPinned = room.isPinned(hit.view);
                    const run: Run = {
                        controller,
                        binding,
                        state,
                        view: hit.view,
                        hit: hit.point,
                        last: now,
                        wasPinned,
                    };
                    this.#runs.set(controller, run);
                    // Whatever the binding does to the panel, the ring must not put it back a frame later.
                    room.pin(hit.view);
                    // The drag owns its buttons from here: the program stops passing them to the window under the ray.
                    // A claim that cannot be made ends the drag at once, so a binding never runs half-claimed.
                    if (this.#hook(`${binding.id}.claim`, () => controller.claim(binding, binding.claims()))) {
                        break;
                    }
                    this.#finish(run, "onCancel", session, room, now);
                }
            }
            if (!this.#runs.has(controller)) {
                this.#declined.add(controller);
            }
        }
        // A controller that left the session while it held a drag.
        for (const run of [...this.#runs.values()]) {
            if (!session.controllers.get(run.controller.id)) {
                this.#finish(run, "onCancel", session, room, now);
            }
        }
    }
}
