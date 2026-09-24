import Clutter from "gi://Clutter";
import Gio from "gi://Gio";
import GLib from "gi://GLib";
import Meta from "gi://Meta";
import Shell from "gi://Shell";
import Cogl from "gi://Cogl";
import { WindowSwitcherFallback } from "./lib/window-switcher-fallback.js";
import { UiSessions } from "./lib/ui-sessions.js";
import { LayerCompanions } from "./lib/layer-companions.js";
import { WindowSnap } from "./lib/window-snap.js";
import { BlurRegions } from "./lib/blur-regions.js";
import { FullscreenReturnGuard } from "./lib/fullscreen-return-guard.js";

import * as Main from "resource:///org/gnome/shell/ui/main.js";
import * as Config from "resource:///org/gnome/shell/ui/components/gnoblinConfig.js";
import * as SessionLock from "resource:///org/gnome/shell/ui/components/gnoblinSessionLock.js";
import * as Animation from "resource:///org/gnome/shell/ui/components/gnoblinAnimation.js";

Gio._promisify(Shell.Screenshot, "composite_to_stream");

// Generic shortcut sessions and window management. Clients own presentation,
// ordering and action semantics.
// Newline-delimited JSON stays on one persistent, user-private Unix socket.
export class CompositorBridge {
    constructor() {
        this.clients = new Set();
        this.clientTokens = new Map();
        this.extensionOperations = new Map();
        this.clientClosedHandlers = new Set();
        this.encoder = new TextEncoder();
        this.layerCompanions = new LayerCompanions();
        this.fullscreenReturnGuard = new FullscreenReturnGuard({
            buttonPress: Clutter.EventType.BUTTON_PRESS,
            buttonRelease: Clutter.EventType.BUTTON_RELEASE,
            pointerEvents: [
                Clutter.EventType.BUTTON_PRESS,
                Clutter.EventType.BUTTON_RELEASE,
                Clutter.EventType.MOTION,
                Clutter.EventType.SCROLL,
            ],
            pick: (event) => this.windowAtPointer(event),
            dismiss: (name, done) => this.dismissUiSession(name, "hide", done),
            set: (armed) => Meta.gnoblin_fullscreen_return_guard_set(global.display, armed),
        });
        this.uiSessions = new UiSessions((client, record) => this.send(client, record), {
            update: (requests) => {
                this.layerCompanions.update(requests);
                this.fullscreenReturnGuard.update([...this.uiSessions.owners], requests);
            },
            cancelDismiss: () => this.layerCompanions.cancelDismiss(),
        });
        this.blurRegions = new BlurRegions();
        this.actions = new Map();
        this.active = null;
        this.grab = null;
        this.timeout = 0;
        this.modifierCheck = 0;
        this.modifierPoll = 0;
        this.windows = new Map();
        this.animationPreviews = new Map();
        this.setupPrivacy();
        this.windowSnap = new WindowSnap(this);
        const directory = GLib.build_filenamev([GLib.get_user_runtime_dir(), "gnoblin"]);
        GLib.mkdir_with_parents(directory, 0o700);
        this.path = Gio.File.new_for_path(
            GLib.getenv("GNOBLIN_COMPOSITOR_SOCKET") || `${directory}/compositor-v1.sock`,
        );
        if (this.path.query_exists(null)) this.path.delete(null);
        this.service = new Gio.SocketService();
        this.service.add_address(
            Gio.UnixSocketAddress.new(this.path.get_path()),
            Gio.SocketType.STREAM,
            Gio.SocketProtocol.DEFAULT,
            null,
        );
        this.service.connect("incoming", (_service, connection) => {
            this.accept(connection);
            return true;
        });
        this.service.start();
        this.accelerator = global.display.connect("accelerator-activated", (_display, action) => this.activate(action));
        this.overlayKey = global.display.connect("overlay-key", () => this.activate("overlay-key"));
        this.capture = global.stage.connect("event", (_stage, event) => this.event(event));
        this.returnClickCapture = global.stage.connect("captured-event", (_stage, event) =>
            this.fullscreenReturnGuard.handle(event) ? Clutter.EVENT_STOP : Clutter.EVENT_PROPAGATE,
        );
        this.session = Main.sessionMode.connect("updated", () => {
            if (SessionLock.isLocked(Main.sessionMode.isLocked)) {
                this.windowSnap.cancel();
                this.end("cancelled");
                this.cancelAnimationPreviews();
            }
        });
        // Native lock state is read synchronously from Mutter. Poll it only at
        // existing event boundaries; the compositor remains the enforcement
        // point and the fallback is the stock Shell state until capability v1.
        this.map = global.window_manager.connect("map", (_manager, actor) => this.track(actor.meta_window));
        this.windowMenu = global.window_manager.connect("show-window-menu", (_manager, window, type, rect) => {
            if (type === Meta.WindowMenuType.WM && Config.settings["window-menu"].length)
                this.showWindowMenu(window, rect.x, rect.y);
        });
        this.focus = global.display.connect("notify::focus-window", () => {
            this.publishWindows();
        });
        for (const actor of global.get_window_actors()) this.track(actor.meta_window);
        this.switcherFallback = new WindowSwitcherFallback(this);
    }

    registerScriptOperation(operation, handler) {
        if (
            typeof operation !== "string" ||
            !/^[a-z][a-z0-9-]*\.[a-z][a-z0-9-]*$/.test(operation) ||
            typeof handler !== "function"
        )
            throw new Error("Compositor script operations must use a namespaced name and a function handler");
        if (this.extensionOperations.has(operation))
            throw new Error(`Compositor operation already registered: ${operation}`);
        this.extensionOperations.set(operation, handler);
        return () => {
            if (this.extensionOperations.get(operation) === handler) this.extensionOperations.delete(operation);
        };
    }

    onClientClosed(handler) {
        if (typeof handler !== "function") throw new TypeError("client close handler must be a function");
        this.clientClosedHandlers.add(handler);
        return () => this.clientClosedHandlers.delete(handler);
    }

    extensionContext(client) {
        if (client.extensionContext) return client.extensionContext;
        client.token = Object.freeze({});
        this.clientTokens.set(client.token, client);
        client.extensionContext = Object.freeze({
            client: client.token,
            pid: client.connection.get_socket().get_credentials().get_unix_pid(),
            isOpen: () => !client.closed,
            send: (record) => this.send(client, record),
            sendTo: (token, record) => {
                const peer = this.clientTokens.get(token);
                if (!peer || peer.closed) return false;
                this.send(peer, record);
                return true;
            },
            broadcast: (record, recipients = null) => {
                if (recipients === null) {
                    this.sendToSubscribers(record, () => true);
                    return;
                }
                const targets = new Set(recipients);
                this.sendToSubscribers(record, (peer) => targets.has(peer.token));
            },
        });
        return client.extensionContext;
    }

    accept(connection) {
        if (this.clients.size >= 32) {
            connection.close(null);
            return;
        }
        const client = {
            connection,
            cancel: new Gio.Cancellable(),
            buffer: new Uint8Array(),
            queue: [],
            queuedBytes: 0,
            writing: false,
            bindings: new Map(),
            decoder: new TextDecoder("utf-8", { fatal: true }),
            closed: false,
        };
        this.clients.add(client);
        this.send(client, {
            event: "hello",
            version: 1,
            features: [
                "ui-sessions",
                "switcher-fallback",
                "overlay-shortcut",
                ...(typeof Shell.BlurEffect.prototype.set_region === "function" ? ["blur-regions"] : []),
                ...(typeof Shell.BlurEffect.prototype.uses_surface_fade === "function" && Config.layerAnimation
                    ? ["layer-animation-policy"]
                    : []),
            ],
        });
        this.read(client);
    }

    send(client, record) {
        if (client.fallback || client.closed) return;
        this.enqueue(client, this.encoder.encode(JSON.stringify(record) + "\n"));
    }

    sendToSubscribers(record, subscribed) {
        let bytes = null;
        for (const client of this.clients) {
            if (client.fallback || client.closed || !subscribed(client)) continue;
            if (!bytes) bytes = this.encoder.encode(JSON.stringify(record) + "\n");
            this.enqueue(client, bytes);
        }
    }

    enqueue(client, bytes) {
        if (client.fallback || client.closed) return;
        if (client.queue.length >= 64) {
            this.close(client);
            return;
        }
        if (client.queuedBytes + bytes.length > 4 * 1024 * 1024) {
            this.close(client);
            return;
        }
        client.queuedBytes += bytes.length;
        client.queue.push(bytes);
        this.write(client);
    }

    write(client) {
        if (client.closed || client.writing || !client.queue.length) return;
        client.writing = true;
        client.connection
            .get_output_stream()
            .write_all_async(client.queue[0], GLib.PRIORITY_DEFAULT, client.cancel, (stream, result) => {
                try {
                    stream.write_all_finish(result);
                } catch (error) {
                    if (!client.closed) console.warn(`gnoblin-compositor write: ${error.message}`);
                    this.close(client);
                    return;
                }
                client.queuedBytes -= client.queue.shift().length;
                client.writing = false;
                this.write(client);
            });
    }

    read(client) {
        client.connection
            .get_input_stream()
            .read_bytes_async(4096, GLib.PRIORITY_DEFAULT, client.cancel, (stream, result) => {
                try {
                    const bytes = stream.read_bytes_finish(result).toArray();
                    if (!bytes.length) {
                        this.close(client);
                        return;
                    }
                    if (client.buffer.length + bytes.length > 16384) throw new Error("input record too large");
                    const buffer = new Uint8Array(client.buffer.length + bytes.length);
                    buffer.set(client.buffer);
                    buffer.set(bytes, client.buffer.length);
                    client.buffer = buffer;
                    let offset = 0;
                    let newline;
                    while ((newline = client.buffer.indexOf(10, offset)) >= 0) {
                        const record = JSON.parse(client.decoder.decode(client.buffer.subarray(offset, newline)));
                        offset = newline + 1;
                        try {
                            this.command(client, record);
                        } catch (error) {
                            this.send(client, { event: "error", id: record.id, message: error.message });
                        }
                    }
                    if (offset) client.buffer = client.buffer.slice(offset);
                    if (!client.closed) this.read(client);
                } catch (error) {
                    if (!client.closed) console.warn(`gnoblin-compositor read: ${error.message}`);
                    this.close(client);
                }
            });
    }

    command(client, record) {
        if (record.op === "ui-session") {
            this.uiSessions.command(client, record);
            return;
        }
        if (record.op === "layer-animation-policy") {
            if (typeof record.namespace !== "string" || record.namespace.length > 128 || !Config.layerAnimation)
                throw new Error("Invalid layer animation query");
            const properties = { type: "layer", layer: record.namespace, title: "", "app-id": "", focused: false };
            this.send(client, {
                event: "layer-animation-policy",
                namespace: record.namespace,
                enter: Config.layerAnimation(properties, true),
                exit: Config.layerAnimation(properties, false),
                windowShadow: Config.windowEffects({ type: "window", title: "", "app-id": "", focused: true }).corners
                    .shadow,
            });
            return;
        }
        if (record.op === "blur-region") {
            this.blurRegions.update(client, record);
            return;
        }
        if (record.op === "shortcut-input") {
            if (
                SessionLock.isLocked(Main.sessionMode.isLocked) ||
                typeof record.name !== "string" ||
                !["prepared", "ready", "closed"].includes(record.state)
            )
                throw new Error("Invalid input handoff");
            const handoff = Main.componentManager?._allComponents?.gnoblinControl?._shortcutInput;
            if (record.state === "prepared") handoff?.prepared(record.name);
            else if (record.state === "ready") handoff?.complete(record.name);
            else handoff?.closed(record.name);
            return;
        }
        if (record.op === "command") {
            if (typeof record.id !== "string" || !/^[\w-]{1,64}$/.test(record.id))
                throw new Error("invalid request ID");
            this.send(client, { event: "reply", id: record.id, result: this.control(record) });
            return;
        }
        const extensionHandler = this.extensionOperations.get(record.op);
        if (extensionHandler) {
            extensionHandler(record, this.extensionContext(client));
            return;
        }
        if (typeof record.op === "string" && record.op.includes("."))
            throw new Error(`Unsupported compositor operation: ${record.op}`);
        if (record.op === "status") {
            this.send(client, {
                event: "status",
                bindings: [...this.actions.values()].map((binding) => binding.id),
                active: this.active?.id ?? null,
            });
            return;
        }
        if (record.op === "window-drag") {
            this.windowSnap.subscribe(client);
            return;
        }
        if (record.op === "snap-offer") {
            this.windowSnap.offer(client, record);
            return;
        }
        if (record.op === "snap-context") {
            const window = global.display.focus_window;
            if (
                SessionLock.isLocked(Main.sessionMode.isLocked) ||
                !window ||
                !this.eligible(window) ||
                !window.allows_resize()
            )
                throw new Error("Focus a resizable window to choose a snap region");
            const monitor = Main.layoutManager.monitors[window.get_monitor()];
            const area = window.get_workspace().get_work_area_for_monitor(monitor.index);
            this.send(client, {
                event: "snap-context",
                window: String(window.get_stable_sequence()),
                monitor: {
                    id: monitor.index,
                    x: monitor.x,
                    y: monitor.y,
                    width: monitor.width,
                    height: monitor.height,
                },
                area: { x: area.x, y: area.y, width: area.width, height: area.height },
            });
            return;
        }
        if (record.op === "snap-window") {
            const window = this.windows.get(record.window)?.window;
            this.windowSnap.apply(window, record.target, record.monitor);
            return;
        }
        if (record.op === "privacy") {
            client.trackPrivacy = true;
            this.publishPrivacy(client);
            return;
        }
        if (record.op === "stop-sharing" || record.op === "stop-recording") {
            const recording = record.op === "stop-recording";
            for (const [handle, state] of this.remoteHandles) if (state.recording === recording) handle.stop();
            return;
        }
        if (record.op === "windows") {
            client.trackWindows = true;
            this.publishWindows(client);
            return;
        }
        if (record.op === "preview") {
            if (
                typeof record.window !== "string" ||
                !Number.isInteger(record.width) ||
                !Number.isInteger(record.height) ||
                record.width < 1 ||
                record.width > 480 ||
                record.height < 1 ||
                record.height > 320
            )
                throw new Error("invalid preview request");
            if (client.preview || client.previewBusy) throw new Error("preview already pending");
            // Give keyboard input precedence over thumbnail readback.
            client.preview = GLib.timeout_add(GLib.PRIORITY_LOW, 32, () => {
                client.preview = 0;
                this.preview(client, record);
                return GLib.SOURCE_REMOVE;
            });
            return;
        }
        if (record.op === "activate") {
            if (!this.switcherFallback.accept(client, record.session)) return;
            const entry = this.windows.get(record.window);
            if (!entry || !this.eligible(entry.window)) throw new Error("window no longer available");
            if (this.active?.client === client) this.end("cancelled");
            Main.activateWindow(entry.window, global.get_current_time());
            return;
        }
        if (record.op === "end") {
            if (this.active?.fallback && record.session !== this.active.session) return;
            if (this.active?.client === client) this.end("cancelled");
            return;
        }
        if (record.op === "clear") {
            this.clear(client);
            return;
        }
        if (
            record.op !== "bind" ||
            typeof record.id !== "string" ||
            !/^[\w-]{1,64}$/.test(record.id) ||
            typeof record.accelerator !== "string" ||
            record.accelerator.length > 128 ||
            !Number.isInteger(record.hold) ||
            ![
                0,
                Clutter.ModifierType.MOD1_MASK,
                Clutter.ModifierType.SUPER_MASK,
                Clutter.ModifierType.CONTROL_MASK,
            ].includes(record.hold) ||
            (record.modal !== undefined && typeof record.modal !== "boolean") ||
            (record.captureInput !== undefined && typeof record.captureInput !== "boolean") ||
            (record.accelerator === "Super" && record.hold !== 0) ||
            client.bindings.size >= 32 ||
            client.bindings.has(record.id)
        )
            throw new Error("invalid shortcut registration");
        const reserved = this.switcherFallback.claim(client, record);
        const overlay = record.accelerator === "Super";
        if (overlay && this.actions.has("overlay-key")) throw new Error("shortcut already claimed: Super");
        const action = overlay
            ? "overlay-key"
            : reserved?.action || global.display.grab_accelerator(record.accelerator, Meta.KeyBindingFlags.NONE);
        if (action === Meta.KeyBindingAction.NONE) throw new Error(`shortcut already claimed: ${record.accelerator}`);
        const binding = reserved || {
            client,
            id: record.id,
            hold: record.hold,
            modal: record.modal !== false,
            captureInput: record.captureInput === true,
            action,
        };
        client.bindings.set(record.id, binding);
        this.actions.set(action, binding);
        Main.wm.allowKeybinding(
            overlay ? action : Meta.external_binding_name_for_action(action),
            Shell.ActionMode.NORMAL | (overlay ? Shell.ActionMode.POPUP : 0),
        );
        this.send(client, { event: "bound", id: record.id });
    }

    activate(action) {
        const binding = this.actions.get(action);
        if (!binding || SessionLock.isLocked(Main.sessionMode.isLocked)) return;
        if (binding.captureInput)
            Main.componentManager?._allComponents?.gnoblinControl?._shortcutInput?.begin(binding.id);
        if (this.active && this.active.client !== binding.client) this.end("cancelled");
        const first = !this.active;
        if (first && binding.hold) {
            this.active = { ...binding, focusWindow: global.display.focus_window };
            if (binding.modal) {
                // Input-only sessions need the stage grab so they receive
                // navigation and release before their surface maps.
                this.grab = Main.pushModal(global.stage, { actionMode: Shell.ActionMode.POPUP });
                if (!(this.grab.get_seat_state() & Clutter.GrabState.KEYBOARD)) {
                    this.end("cancelled");
                    return;
                }
            }
            this.timeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 10000, () => {
                this.timeout = 0;
                this.end("cancelled");
                return GLib.SOURCE_REMOVE;
            });
            if (!binding.modal) this.startPassiveModifierPoll();
        }
        const session = this.switcherFallback.step(binding, first);
        if (this.active) this.active.session = session;
        this.send(binding.client, {
            event: "activated",
            id: binding.id,
            first,
            session,
            modifiers: global.get_pointer()[2],
            time: global.get_current_time(),
        });
        // A release may precede the client receiving activation. Send both
        // records in order instead of waiting for the client to map a surface.
        if (this.active && !this.modifiersHeld(global.get_pointer()[2], this.active.hold)) this.checkModifiers();
    }

    modifiersHeld(state, hold) {
        // Mutter reports the physical Super key as MOD4 in pointer/event
        // state, while grab_accelerator uses Clutter's SUPER_MASK. Accept both
        // representations so a held Super chord does not finish immediately.
        if (hold === Clutter.ModifierType.SUPER_MASK)
            return Boolean(state & (Clutter.ModifierType.SUPER_MASK | Clutter.ModifierType.MOD4_MASK));
        return Boolean(state & hold);
    }

    startPassiveModifierPoll() {
        if (this.modifierPoll) return;
        this.modifierPoll = GLib.timeout_add(GLib.PRIORITY_HIGH_IDLE, 16, () => {
            if (!this.active || this.active.modal) {
                this.modifierPoll = 0;
                return GLib.SOURCE_REMOVE;
            }
            if (!this.modifiersHeld(global.get_pointer()[2], this.active.hold)) {
                this.modifierPoll = 0;
                this.end("released");
                return GLib.SOURCE_REMOVE;
            }
            return GLib.SOURCE_CONTINUE;
        });
    }

    checkModifiers() {
        if (this.modifierCheck) return;
        this.modifierCheck = GLib.idle_add(GLib.PRIORITY_HIGH_IDLE, () => {
            this.modifierCheck = 0;
            if (this.active && !this.modifiersHeld(global.get_pointer()[2], this.active.hold)) this.end("released");
            return GLib.SOURCE_REMOVE;
        });
    }

    event(event) {
        if (!this.active) return Clutter.EVENT_PROPAGATE;
        const type = event.type();
        if (!this.active.modal) {
            // A passive hold (for example Super+Space layout cycling) only
            // observes modifier release. It must never take focus or swallow
            // the application's pointer and keyboard input.
            if (type === Clutter.EventType.KEY_RELEASE) {
                this.checkModifiers();
                // Mutter treats a propagated Super release as the overlay key
                // even after a passive chord. This release belongs to the
                // shortcut; consuming only it keeps the focused app intact.
                if (event.get_key_symbol() === Clutter.KEY_Super_L || event.get_key_symbol() === Clutter.KEY_Super_R)
                    return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        }
        if (type === Clutter.EventType.BUTTON_PRESS) {
            if (this.active.client.fallback) {
                this.end("cancelled");
                return Clutter.EVENT_STOP;
            }
            const [x, y] = event.get_coords();
            this.send(this.active.client, { event: "pointer", x, y, button: event.get_button() });
            return Clutter.EVENT_STOP;
        }
        if (type !== Clutter.EventType.KEY_PRESS && type !== Clutter.EventType.KEY_RELEASE)
            return Clutter.EVENT_PROPAGATE;
        if (type === Clutter.EventType.KEY_RELEASE) {
            // Event dispatch can precede Mutter's modifier-state update.
            // Check after dispatch, including when either left/right modifier
            // remains down. Never require a client surface to receive release.
            this.checkModifiers();
            return Clutter.EVENT_PROPAGATE;
        }
        // Queued presses retain their own modifier state. The current device
        // state may already reflect a later release in the same event batch.
        if (!this.modifiersHeld(event.get_state(), this.active.hold)) {
            this.end("released");
            return Clutter.EVENT_PROPAGATE;
        }
        if (type === Clutter.EventType.KEY_PRESS) {
            if (this.active.fallback && this.switcherFallback.key(event.get_key_symbol())) return Clutter.EVENT_STOP;
            const action = global.display.get_keybinding_action(event.get_key_code(), event.get_state());
            if (this.actions.has(action)) this.activate(action);
            else
                this.send(this.active.client, {
                    event: "key",
                    key: event.get_key_symbol(),
                    modifiers: event.get_state(),
                });
        }
        return Clutter.EVENT_STOP;
    }

    end(reason) {
        if (this.modifierCheck) GLib.source_remove(this.modifierCheck);
        this.modifierCheck = 0;
        if (this.modifierPoll) GLib.source_remove(this.modifierPoll);
        this.modifierPoll = 0;
        if (this.timeout) GLib.source_remove(this.timeout);
        this.timeout = 0;
        if (!this.active) return;
        const active = this.active;
        this.active = null;
        if (this.grab) {
            const grab = this.grab;
            this.grab = null;
            Main.popModal(grab);
            if (
                !SessionLock.isLocked(Main.sessionMode.isLocked) &&
                active.focusWindow?.get_compositor_private() &&
                !active.focusWindow.minimized
            )
                active.focusWindow.focus(global.get_current_time());
        }
        this.send(active.client, { event: reason, session: active.session || 0 });
        this.switcherFallback.end(active, reason);
    }

    clear(client) {
        this.switcherFallback.close(client);
        if (this.active?.client === client) this.end("cancelled");
        for (const binding of client.bindings.values()) {
            if (binding.captureInput)
                Main.componentManager?._allComponents?.gnoblinControl?._shortcutInput?.closed(binding.id);
            if (this.switcherFallback.release(binding)) continue;
            if (binding.action !== "overlay-key") global.display.ungrab_accelerator(binding.action);
            Main.wm.allowKeybinding(
                binding.action === "overlay-key"
                    ? binding.action
                    : Meta.external_binding_name_for_action(binding.action),
                Shell.ActionMode.NONE,
            );
            this.actions.delete(binding.action);
        }
        client.bindings.clear();
    }

    eligible(window) {
        return window && !window.skip_taskbar && !window.is_override_redirect();
    }

    setupPrivacy() {
        // Active handles and their start times live on the session global, so
        // replacing the bridge transport does not reset elapsed time. Each
        // handle's stopped callback persists until that session ends.
        this.remoteHandles = global.__gnoblinRemoteAccessHandles ??= new Map();
        global.__gnoblinPublishPrivacy = () => this.publishPrivacy();
        this.remoteController = global.backend.get_remote_access_controller();
        this.remoteSignal = this.remoteController?.connect("new-handle", (_controller, handle) =>
            this.trackRemote(handle),
        );
        const panel = Main.panel?.statusArea;
        for (const handles of [panel?.screenSharing?._handles, panel?.quickSettings?._remoteAccess?._handles])
            for (const handle of handles ?? []) this.trackRemote(handle);
        this.ownsCameraMonitor = !panel?.quickSettings?._camera?._cameraMonitor;
        this.cameraMonitor = panel?.quickSettings?._camera?._cameraMonitor ?? new Shell.CameraMonitor();
        this.cameraSignal = this.cameraMonitor.connect("notify::cameras-in-use", () => this.publishPrivacy());
    }

    trackRemote(handle) {
        if (this.remoteHandles.has(handle)) return;
        const state = {
            recording: Boolean(handle.is_recording ?? handle.isRecording),
            started: GLib.get_monotonic_time(),
        };
        this.remoteHandles.set(handle, state);
        state.signal = handle.connect("stopped", () => {
            global.__gnoblinRemoteAccessHandles.delete(handle);
            handle.disconnect(state.signal);
            global.__gnoblinPublishPrivacy?.();
        });
        this.publishPrivacy();
    }

    publishPrivacy(client = null) {
        const states = [...this.remoteHandles.values()];
        const recordings = states.filter((state) => state.recording);
        const started = recordings.length ? Math.min(...recordings.map((state) => state.started)) : 0;
        const record = {
            event: "privacy",
            screenSharing: states.some((state) => !state.recording),
            recording: recordings.length > 0,
            recordingCount: recordings.length,
            recordingElapsed: started ? Math.floor((GLib.get_monotonic_time() - started) / 1000000) : 0,
            cameraInUse: Boolean(this.cameraMonitor?.cameras_in_use),
            locationCaptures: global.__gnoblinLocationCaptures?.() ?? [],
        };
        if (client) this.send(client, record);
        else this.sendToSubscribers(record, (peer) => peer.trackPrivacy);
    }

    async preview(client, request) {
        client.previewBusy = true;
        const stream = Gio.MemoryOutputStream.new_resizable();
        try {
            if (SessionLock.isLocked(Main.sessionMode.isLocked)) throw new Error("session locked");
            const window = this.windows.get(request.window)?.window;
            if (!this.eligible(window)) throw new Error("window no longer available");
            // Keep full-size pixels on the GPU. Only the small render target is
            // read back; Shell's PNG encoder runs asynchronously on a worker.
            const actor = window.get_compositor_private();
            // Occlusion can make actor painting transparent, including when
            // the chooser covers a mapped window. Sample the backing buffer
            // directly where possible; it also survives minimisation.
            const backing = actor?.get_texture()?.get_texture();
            const source =
                backing && (!backing.is_simple || backing.is_simple())
                    ? backing.get_plane
                        ? backing.get_plane(0)
                        : backing
                    : actor?.paint_to_content(null)?.get_texture();
            if (!source) throw new Error("window has no image");
            const scale = Math.min(1, request.width / source.get_width(), request.height / source.get_height());
            const width = Math.max(1, Math.round(source.get_width() * scale));
            const height = Math.max(1, Math.round(source.get_height() * scale));
            const context = source.get_context();
            const texture = Cogl.Texture2D.new_with_size(context, width, height);
            const framebuffer = Cogl.Offscreen.new_with_texture(texture);
            framebuffer.allocate();
            framebuffer.orthographic(0, 0, width, height, -1, 1);
            const pipeline = Cogl.Pipeline.new(context);
            pipeline.set_layer_texture(0, source);
            pipeline.set_layer_filters(0, Cogl.PipelineFilter.LINEAR, Cogl.PipelineFilter.LINEAR);
            pipeline.set_blend("RGBA = ADD (SRC_COLOR, 0)");
            framebuffer.draw_rectangle(pipeline, 0, 0, width, height);
            // composite_to_stream reads a subtexture. Flush this render target
            // explicitly; the subtexture does not own its pending draw journal.
            framebuffer.flush();
            const pixbuf = await Shell.Screenshot.composite_to_stream(
                texture,
                0,
                0,
                width,
                height,
                1,
                null,
                0,
                0,
                1,
                stream,
            );
            if (pixbuf.get_has_alpha()) {
                const pixels = pixbuf.get_pixels();
                const stride = pixbuf.get_rowstride();
                const channels = pixbuf.get_n_channels();
                let visible = false;
                for (let y = 0; y < height && !visible; y++)
                    for (let x = 0; x < width; x++)
                        if (pixels[y * stride + x * channels + channels - 1]) {
                            visible = true;
                            break;
                        }
                if (!visible) throw new Error("window image buffer unavailable");
            }
            stream.close(null);
            if (client.closed || SessionLock.isLocked(Main.sessionMode.isLocked) || !this.windows.has(request.window))
                return;
            const bytes = stream.steal_as_bytes().toArray();
            this.send(client, {
                event: "preview",
                window: request.window,
                width,
                height,
                source: `data:image/png;base64,${GLib.base64_encode(bytes)}`,
            });
        } catch (error) {
            this.send(client, { event: "preview", window: request.window, source: "", message: error.message });
        } finally {
            stream.close(null);
            client.previewBusy = false;
        }
    }

    dismissUiSession(name, action, done = () => {}) {
        const owner = this.uiSessions.owners.get(name);
        const state = owner?.state;
        if (!state?.revealCompanions || typeof state.surface !== "string") {
            done();
            return;
        }
        this.layerCompanions.dismiss(state.surface, () => {
            this.uiSessions.command(null, { action: "command", name, command: { action } });
            done();
        });
    }

    dismissRevealedUi() {
        const [name, owner] =
            [...this.uiSessions.owners].find(
                ([, candidate]) => candidate.state?.revealCompanions && typeof candidate.state.surface === "string",
            ) ?? [];
        if (name) this.dismissUiSession(name, "hide");
    }

    windowAtPointer(event) {
        const [x, y] = event.get_coords();
        let actor = global.stage.get_actor_at_pos(Clutter.PickMode.REACTIVE, x, y);
        while (actor && !actor.meta_window) actor = actor.get_parent();
        return actor?.meta_window || null;
    }

    track(window) {
        if (!window) return;
        const actor = window.get_compositor_private();
        if (actor) this.blurRegions.apply(actor);
        const id = String(window.get_stable_sequence());
        if (this.windows.has(id)) return;
        const signals = ["notify::title", "notify::minimized", "notify::skip-taskbar"].map((signal) =>
            window.connect(signal, () => this.publishWindows()),
        );
        signals.push(
            window.connect("raised", () => {
                if (!window.is_fullscreen()) return;
                const [x, y, modifiers] = global.get_pointer();
                const buttons =
                    Clutter.ModifierType.BUTTON1_MASK |
                    Clutter.ModifierType.BUTTON2_MASK |
                    Clutter.ModifierType.BUTTON3_MASK;
                if (!(modifiers & buttons)) return;
                let actor = global.stage.get_actor_at_pos(Clutter.PickMode.REACTIVE, x, y);
                while (actor && !actor.meta_window) actor = actor.get_parent();
                if (actor?.meta_window === window) this.dismissRevealedUi();
            }),
        );
        signals.push(
            window.connect("notify::fullscreen", () => {
                if (window.is_fullscreen() && this.eligible(window)) this.dismissRevealedUi();
                this.publishWindows();
            }),
        );
        signals.push(
            window.connect("unmanaged", () => {
                this.windowSnap.forget(id);
                this.cancelPreviewsForTarget(id);
                this.windows.delete(id);
                for (const signal of signals) window.disconnect(signal);
                this.publishWindows();
            }),
        );
        this.windows.set(id, { window, signals });
        this.publishWindows();
    }

    windowRecords() {
        const tracker = Shell.WindowTracker.get_default();
        return [...this.windows]
            .filter(([_id, entry]) => this.eligible(entry.window))
            .map(([id, { window }]) => {
                const monitor = Main.layoutManager.monitors[window.get_monitor()];
                const frame = window.get_frame_rect();
                const app = tracker.get_window_app(window);
                const gtkAppId = window.get_gtk_application_id() || "";
                const wmClass = window.get_wm_class() || "";
                return {
                    id,
                    title: window.title || "",
                    appId: app && !app.is_window_backed() ? app.get_id() : window.get_wm_class() || "",
                    gtkAppId,
                    wmClass,
                    ruleAppId: gtkAppId || wmClass,
                    focused: global.display.focus_window === window,
                    minimized: window.minimized,
                    workspace: window.get_workspace()?.index() + 1 || null,
                    monitorIndex: window.get_monitor(),
                    maximized: window.get_maximize_flags() === Meta.MaximizeFlags.BOTH,
                    fullscreen: window.is_fullscreen(),
                    geometry: { x: frame.x, y: frame.y, width: frame.width, height: frame.height },
                    lastUserTime: window.get_user_time(),
                    parent: window.get_transient_for()
                        ? String(window.get_transient_for().get_stable_sequence())
                        : null,
                    monitor: monitor ? { x: monitor.x, y: monitor.y } : null,
                };
            });
    }

    control(record) {
        if (record.command === "animation") return this.animationCommand(record);
        if (record.command === "layers") return { surfaces: this.layerRecords() };
        if (record.command === "capture-windows") {
            if (SessionLock.isLocked(Main.sessionMode.isLocked)) throw new Error("Session is locked.");
            const windows = global.display.sort_windows_by_stacking(
                global.get_window_actors().map((actor) => actor.meta_window),
            );
            return {
                windows: windows
                    .reverse()
                    .filter((window) => this.eligible(window) && !window.minimized && window.showing_on_its_workspace())
                    .map((window) => {
                        const frame = window.get_frame_rect();
                        const actor = window.get_compositor_private();
                        const paintBoxResult = actor?.get_paint_box();
                        const paintBox = Array.isArray(paintBoxResult) ? paintBoxResult[1] : paintBoxResult;
                        const paintWidth =
                            paintBox && Number.isFinite(paintBox.x1) && Number.isFinite(paintBox.x2)
                                ? Math.max(1, Math.ceil(paintBox.x2 - paintBox.x1))
                                : 0;
                        const paintHeight =
                            paintBox && Number.isFinite(paintBox.y1) && Number.isFinite(paintBox.y2)
                                ? Math.max(1, Math.ceil(paintBox.y2 - paintBox.y1))
                                : 0;
                        const app = Shell.WindowTracker.get_default().get_window_app(window);
                        const texture = actor?.get_texture()?.get_texture();
                        return {
                            // Window capture follows the compositor paint box,
                            // which includes Gnoblin decoration children such
                            // as the shadow outside the client allocation.
                            bufferWidth: paintWidth || texture?.get_width() || frame.width,
                            bufferHeight: paintHeight || texture?.get_height() || frame.height,
                            id: String(window.get_id()),
                            title: window.title || app?.get_name() || "",
                            appId: app?.get_id() || "",
                            appName: app?.get_name() || "",
                            x: frame.x,
                            y: frame.y,
                            width: frame.width,
                            height: frame.height,
                        };
                    }),
            };
        }
        if (record.command === "windows") return { windows: this.windowRecords() };
        const manager = global.workspace_manager;
        if (record.command === "workspaces")
            return {
                workspaces: Array.from({ length: manager.n_workspaces }, (_, index) => ({
                    id: index + 1,
                    active: index === manager.get_active_workspace_index(),
                    windows: manager
                        .get_workspace_by_index(index)
                        .list_windows()
                        .filter((window) => this.eligible(window)).length,
                })),
            };
        if (record.command === "monitors")
            return {
                monitors: Main.layoutManager.monitors.map((monitor) => ({
                    id: monitor.index,
                    x: monitor.x,
                    y: monitor.y,
                    width: monitor.width,
                    height: monitor.height,
                    primary: monitor.index === Main.layoutManager.primaryIndex,
                    scale: global.display.get_monitor_scale(monitor.index),
                })),
            };
        if (SessionLock.isLocked(Main.sessionMode.isLocked))
            throw new Error("window management is unavailable while the session is locked");
        const workspace = () => {
            if (!Number.isInteger(record.workspace) || record.workspace < 1 || record.workspace > manager.n_workspaces)
                throw new Error("workspace not found; list workspaces first");
            return manager.get_workspace_by_index(record.workspace - 1);
        };
        if (record.command === "workspace-switch") {
            workspace().activate(global.get_current_time());
            return { ok: true, pending: true, workspace: record.workspace };
        }
        if (record.command !== "window") throw new Error("unknown compositor command");
        const actions = [
            "menu",
            "interactive-move",
            "interactive-resize",
            "above",
            "unabove",
            "stick",
            "unstick",
            "focus",
            "close",
            "minimize",
            "restore-or-minimize",
            "restore",
            "maximize",
            "unmaximize",
            "fullscreen",
            "unfullscreen",
            "move",
            "resize",
            "workspace",
            "monitor",
        ];
        if (!actions.includes(record.action)) throw new Error("unknown window action");
        const window =
            record.window === "active" ? global.display.focus_window : this.windows.get(record.window)?.window;
        if (!window || !this.eligible(window)) throw new Error("window no longer available; list windows first");
        const requireCapability = (allowed, message) => {
            if (!allowed) throw new Error(message);
        };
        switch (record.action) {
            case "menu": {
                const [x, y] = global.get_pointer();
                this.showWindowMenu(window, x, y);
                break;
            }
            case "above":
                window.make_above();
                break;
            case "unabove":
                window.unmake_above();
                break;
            case "stick":
                window.stick();
                break;
            case "unstick":
                window.unstick();
                break;
            case "interactive-move":
            case "interactive-resize": {
                const move = record.action === "interactive-move";
                requireCapability(
                    (move ? window.allows_move() : window.allows_resize()) &&
                        !window.is_fullscreen() &&
                        !window.get_maximize_flags(),
                    "window cannot move or resize in this state",
                );
                Main.activateWindow(window, global.get_current_time());
                const sprite = global.stage.context.get_backend().get_pointer_sprite(global.stage);
                requireCapability(
                    window.begin_grab_op(
                        move ? Meta.GrabOp.KEYBOARD_MOVING : Meta.GrabOp.KEYBOARD_RESIZING_UNKNOWN,
                        sprite,
                        global.get_current_time(),
                        null,
                    ),
                    "could not begin window grab",
                );
                break;
            }
            case "focus":
                Main.activateWindow(window, global.get_current_time());
                break;
            case "close":
                requireCapability(window.can_close(), "window cannot be closed");
                window.delete(global.get_current_time());
                break;
            case "minimize":
                requireCapability(window.can_minimize(), "window cannot be minimized");
                window.minimize();
                break;
            case "restore-or-minimize":
                this.windowSnap.restoreOrMinimize(window);
                break;
            case "restore":
                window.unminimize();
                break;
            case "maximize":
                requireCapability(window.can_maximize(), "window cannot be maximized");
                window.maximize(Meta.MaximizeFlags.BOTH);
                break;
            case "unmaximize":
                window.unmaximize(Meta.MaximizeFlags.BOTH);
                break;
            case "fullscreen":
                window.make_fullscreen();
                break;
            case "unfullscreen":
                window.unmake_fullscreen();
                break;
            case "move":
                requireCapability(
                    window.allows_move() && !window.is_fullscreen() && !window.get_maximize_flags(),
                    "window cannot move; unmaximize or leave fullscreen first",
                );
                if (![record.x, record.y].every((value) => Number.isInteger(value) && Math.abs(value) <= 100000))
                    throw new Error("invalid window position");
                window.move_frame(true, record.x, record.y);
                break;
            case "resize": {
                requireCapability(
                    window.allows_resize() && !window.is_fullscreen() && !window.get_maximize_flags(),
                    "window cannot resize; unmaximize or leave fullscreen first",
                );
                if (
                    ![record.width, record.height].every(
                        (value) => Number.isInteger(value) && value >= 1 && value <= 32768,
                    )
                )
                    throw new Error("invalid window size");
                const frame = window.get_frame_rect();
                window.move_resize_frame(true, frame.x, frame.y, record.width, record.height);
                break;
            }
            case "workspace":
                window.change_workspace(workspace());
                break;
            case "monitor":
                if (
                    !Number.isInteger(record.monitor) ||
                    !Main.layoutManager.monitors.some((monitor) => monitor.index === record.monitor)
                )
                    throw new Error("monitor not found");
                window.move_to_monitor(record.monitor);
                break;
        }
        return { ok: true, pending: true, window: String(window.get_stable_sequence()), action: record.action };
    }

    layerRecords() {
        return [...this.windows]
            .map(([id, { window }]) => ({
                id,
                namespace: Meta.gnoblin_layer_namespace(window),
                title: window.title || "",
            }))
            .filter(({ namespace }) => namespace !== null);
    }

    animationCommand(record) {
        if (SessionLock.isLocked(Main.sessionMode.isLocked))
            throw new Error("animation previews are unavailable while the session is locked");
        const action = record.action;
        if (action === "list") {
            const internalEvents = new Set([
                "console-open",
                "console-close",
                "shadow-change",
                "tile-preview-open",
                "tile-preview-close",
                "dialog-dim",
                "dialog-undim",
                "layer-companion-close",
                "workspace-switch",
            ]);
            const animations = Animation.presets().map((preset) => ({
                ...preset,
                previewable: preset.name !== "none" && !internalEvents.has(preset.event),
            }));
            animations.push(
                ...(Config.settings.animations ?? []).map(({ name, event, duration, ease }) => ({
                    name,
                    event,
                    duration,
                    ease,
                    previewable: duration !== 0 && !internalEvents.has(event),
                })),
            );
            const unique = new Map(
                animations.map((animation) => [`${animation.name}\0${animation.event ?? ""}`, animation]),
            );
            return { animations: [...unique.values()] };
        }
        if (action === "surfaces") return { surfaces: this.layerRecords() };
        if (action === "inspect" || action === "preview") {
            if (
                typeof record.name !== "string" ||
                !/^[a-zA-Z0-9_-]{1,80}$/.test(record.name) ||
                (record.event !== undefined &&
                    (typeof record.event !== "string" || !/^[a-z][a-z0-9-]{0,63}$/.test(record.event)))
            )
                throw new Error("invalid animation name or event");
            const targetType = record.targetType ?? "window";
            let window = null;
            let actor = null;
            if (targetType === "window") {
                window =
                    record.target === "active" ? global.display.focus_window : this.windows.get(record.target)?.window;
            } else if (targetType === "layer" || targetType === "namespace") {
                const candidates = [...this.windows.values()]
                    .map(({ window: candidate }) => candidate)
                    .filter((candidate) => {
                        const namespace = Meta.gnoblin_layer_namespace(candidate);
                        return (
                            namespace !== null &&
                            (targetType === "layer"
                                ? String(candidate.get_stable_sequence()) === String(record.target)
                                : namespace === record.target)
                        );
                    });
                if (candidates.length !== 1)
                    throw new Error(
                        candidates.length
                            ? `layer namespace matches multiple surfaces (${candidates.map((candidate) => candidate.get_stable_sequence()).join(", ")}); use --layer ID`
                            : "layer surface not found",
                    );
                window = candidates[0];
            } else {
                throw new Error("invalid animation target type");
            }
            if (!window) throw new Error("animation target window no longer available");
            const isLayer = Meta.gnoblin_layer_namespace(window) !== null;
            if (window.is_override_redirect() || (window.skip_taskbar && !isLayer))
                throw new Error("animation target window no longer available");
            actor = window.get_compositor_private();
            if (!actor) throw new Error("animation target has no compositor actor");
            const namespace = Meta.gnoblin_layer_namespace(window);
            const properties = Config.windowProperties(window);
            const monitor = Main.layoutManager.monitors[window.get_monitor()] ?? null;
            const frame = window.get_frame_rect();
            const custom = (Config.settings.animations ?? []).find(
                (animation) =>
                    animation.name === record.name && (record.event === undefined || animation.event === record.event),
            );
            const preset = Animation.presets().find((candidate) => candidate.name === record.name);
            if (!custom && !preset) throw new Error(`animation not found: ${record.name}`);
            const supportedEvents = new Set([
                ...Animation.presets()
                    .map((candidate) => candidate.event)
                    .filter(Boolean),
                ...(Config.settings.animations ?? []).map((animation) => animation.event),
            ]);
            const context = {
                ...properties,
                actor,
                window,
                monitor,
                targetGeom: [false, null],
                rtl: Clutter.get_default_text_direction() === Clutter.TextDirection.RTL,
                offset: [0, 0],
            };
            const requestedEvent =
                record.event ?? custom?.event ?? preset?.event ?? (namespace !== null ? "layer-open" : "open");
            if (requestedEvent === "minimize" || requestedEvent === "restore")
                context.targetGeom = Config.minimizeTarget(window, monitor);
            if (namespace !== null && (requestedEvent === "layer-open" || requestedEvent === "layer-close"))
                context.offset = Config.layerOffset(Meta.gnoblin_layer_anchor(window), frame, monitor);
            const spec = Animation.resolve(record.name, requestedEvent, context, custom);
            if (!spec) throw new Error(`animation not found: ${record.name}`);
            const event = spec.event;
            if (!supportedEvents.has(event)) throw new Error(`unsupported animation event: ${event}`);
            if (!Config.animationNameSupports(record.name, [event]))
                throw new Error(`${record.name} does not support ${event}`);
            const exactPreset = Animation.presets().find(
                (candidate) => candidate.name === record.name && candidate.event !== null,
            );
            if (exactPreset && exactPreset.event !== event)
                throw new Error(`${record.name} is for the ${exactPreset.event} event; requested ${event}`);
            if ((event === "layer-open" || event === "layer-close") && namespace === null)
                throw new Error(`${event} requires a layer-shell surface target`);
            if (namespace !== null && (event === "minimize" || event === "restore" || event === "workspace-switch"))
                throw new Error(`${event} previews require a window target`);
            const internalEvents = new Set([
                "console-open",
                "console-close",
                "shadow-change",
                "tile-preview-open",
                "tile-preview-close",
                "dialog-dim",
                "dialog-undim",
                "layer-companion-close",
                "workspace-switch",
            ]);
            if (internalEvents.has(event))
                throw new Error(
                    `${event} targets an internal shell actor and cannot be previewed on a window or layer surface`,
                );
            const matchProperties = { ...properties };
            const publicContext = {
                ...matchProperties,
                actor: {
                    x: actor.x,
                    y: actor.y,
                    width: actor.width,
                    height: actor.height,
                },
                monitor: monitor ? { x: monitor.x, y: monitor.y, width: monitor.width, height: monitor.height } : null,
                targetGeom:
                    context.targetGeom[0] && context.targetGeom[1]
                        ? [
                              true,
                              {
                                  x: context.targetGeom[1].x,
                                  y: context.targetGeom[1].y,
                                  width: context.targetGeom[1].width,
                                  height: context.targetGeom[1].height,
                              },
                          ]
                        : [false, null],
                offset: [...context.offset],
                rtl: context.rtl,
            };
            if (action === "inspect")
                return {
                    name: record.name,
                    event,
                    target: String(window.get_stable_sequence()),
                    properties: matchProperties,
                    context: publicContext,
                    spec,
                };
            if (spec.duration <= 0) throw new Error("animation has zero duration and cannot be stepped");
            if (window.minimized || !actor.visible || !window.showing_on_its_workspace())
                throw new Error("animation preview target must be visible and unminimized");
            for (const [sessionId, current] of this.animationPreviews) {
                if (current.actor === actor) {
                    current.controller.cancel({ restore: true });
                    this.animationPreviews.delete(sessionId);
                }
            }
            const session = GLib.uuid_string_random();
            let entry;
            let controller = null;
            let completed = false;
            controller = Animation.run(actor, spec, {
                paused: !record.autoplay,
                onFrame: () => {},
                onComplete: (finished) => {
                    completed = finished;
                    if (entry && this.animationPreviews.get(session) === entry) this.animationPreviews.delete(session);
                    if (finished) {
                        controller?.cancel({ restore: true });
                        this.sendToSubscribers({ event: "animation-preview-finished", session }, () => true);
                    }
                },
            });
            entry = { controller, actor, target: String(window.get_stable_sequence()), name: record.name, event };
            if (completed) controller.cancel({ restore: true });
            else this.animationPreviews.set(session, entry);
            return { session, target: entry.target, name: record.name, event, paused: !record.autoplay, spec };
        }
        if (typeof record.session !== "string" || !this.animationPreviews.has(record.session))
            throw new Error("animation preview session not found");
        const entry = this.animationPreviews.get(record.session);
        if (action === "seek") {
            if (
                typeof record.progress !== "number" ||
                !Number.isFinite(record.progress) ||
                record.progress < 0 ||
                record.progress > 1
            )
                throw new Error("progress must be between 0 and 1");
            entry.controller.seek(record.progress);
        } else if (action === "step") {
            if (!Number.isInteger(record.milliseconds) || record.milliseconds < 1 || record.milliseconds > 60000)
                throw new Error("step milliseconds out of range");
            entry.controller.step(record.milliseconds);
        } else if (action === "play") entry.controller.play();
        else if (action === "pause") entry.controller.pause();
        else if (action === "stop") {
            entry.controller.cancel({ restore: true });
            this.animationPreviews.delete(record.session);
        } else throw new Error("unknown animation action");
        return { ok: true, session: record.session, action };
    }

    cancelPreviewsForTarget(target) {
        for (const [session, entry] of this.animationPreviews) {
            if (entry.target !== target) continue;
            entry.controller.cancel({ restore: true });
            this.animationPreviews.delete(session);
        }
    }

    cancelAnimationPreviews() {
        for (const [session, entry] of this.animationPreviews) {
            entry.controller.cancel({ restore: true });
            this.animationPreviews.delete(session);
        }
    }

    showWindowMenu(window, x, y) {
        if (SessionLock.isLocked(Main.sessionMode.isLocked) || !window || !this.eligible(window)) return;
        const command = Config.settings["window-menu"];
        if (!command.length) throw new Error("No shell.window-menu command configured");
        const maximized = !!window.get_maximize_flags();
        const fullscreen = window.is_fullscreen();
        const actions = [
            { id: "minimize", text: "Minimize", enabled: window.can_minimize() },
            {
                id: maximized ? "unmaximize" : "maximize",
                text: maximized ? "Restore" : "Maximize",
                enabled: window.can_maximize() && !fullscreen,
            },
            { id: "interactive-move", text: "Move", enabled: window.allows_move() && !maximized && !fullscreen },
            { id: "interactive-resize", text: "Resize", enabled: window.allows_resize() && !maximized && !fullscreen },
            { isSeparator: true },
            {
                id: window.is_above() ? "unabove" : "above",
                text: "Always on Top",
                checked: window.is_above(),
                enabled: !fullscreen,
            },
            {
                id: window.is_on_all_workspaces() ? "unstick" : "stick",
                text: "Always on Visible Workspace",
                checked: window.is_on_all_workspaces(),
                enabled: true,
            },
            { isSeparator: true },
            { id: "close", text: "Close", enabled: window.can_close() },
        ];
        const record = {
            version: 1,
            window: String(window.get_stable_sequence()),
            title: window.title,
            x: Math.round(x),
            y: Math.round(y),
            actions,
        };
        const child = Gio.Subprocess.new([...command, JSON.stringify(record)], Gio.SubprocessFlags.NONE);
        child.wait_check_async(null, (process, result) => {
            try {
                process.wait_check_finish(result);
            } catch (error) {
                console.warn(`gnoblin window menu: ${error.message}`);
            }
        });
    }

    publishWindows(client = null) {
        if (client) {
            this.send(client, { event: "windows", windows: this.windowRecords() });
            return;
        }
        for (const peer of this.clients)
            if (peer.trackWindows) {
                const windows = this.windowRecords();
                this.sendToSubscribers({ event: "windows", windows }, (subscriber) => subscriber.trackWindows);
                return;
            }
    }

    close(client) {
        if (client.closed) return;
        client.closed = true;
        if (client.token) {
            for (const handler of this.clientClosedHandlers) {
                try {
                    handler(client.token);
                } catch (error) {
                    console.warn(`gnoblin-compositor client cleanup: ${error.message}`);
                }
            }
            this.clientTokens.delete(client.token);
        }
        this.uiSessions.close(client);
        this.windowSnap.close(client);
        this.blurRegions.close(client);
        if (client.preview) GLib.source_remove(client.preview);
        client.preview = 0;
        this.clear(client);
        client.cancel.cancel();
        try {
            client.connection.close(null);
        } catch {
            /* Connection already closed. */
        }
        this.clients.delete(client);
    }

    destroy() {
        global.window_manager.disconnect(this.windowMenu);
        this.windowSnap.destroy();
        this.cancelAnimationPreviews();
        this.end("cancelled");
        global.__gnoblinPublishPrivacy = null;
        if (this.remoteSignal) this.remoteController.disconnect(this.remoteSignal);
        this.cameraMonitor.disconnect(this.cameraSignal);
        if (this.ownsCameraMonitor) this.cameraMonitor.run_dispose();
        for (const client of this.clients) this.close(client);
        this.fullscreenReturnGuard.disarm();
        this.layerCompanions.destroy();
        this.switcherFallback.destroy();
        global.display.disconnect(this.accelerator);
        global.display.disconnect(this.overlayKey);
        global.stage.disconnect(this.capture);
        global.stage.disconnect(this.returnClickCapture);
        Main.sessionMode.disconnect(this.session);
        global.window_manager.disconnect(this.map);
        global.display.disconnect(this.focus);
        for (const { window, signals } of this.windows.values())
            for (const signal of signals) window.disconnect(signal);
        this.windows.clear();
        this.extensionOperations.clear();
        this.clientClosedHandlers.clear();
        this.service.stop();
        this.service.close();
        this.path.delete(null);
    }
}
