import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import Cogl from 'gi://Cogl';
import {ClipboardPaste} from './lib/clipboard-paste.js';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

Gio._promisify(Shell.Screenshot, 'composite_to_stream');

// Generic shortcut sessions and window management. Clients own presentation,
// ordering and action semantics.
// Newline-delimited JSON stays on one persistent, user-private Unix socket.
class CompositorBridge {
    constructor() {
        this.clients = new Set();
        this.clipboardPaste = new ClipboardPaste(global.stage.context.get_backend().get_default_seat());
        this.actions = new Map();
        this.active = null;
        this.grab = null;
        this.timeout = 0;
        this.modifierCheck = 0;
        this.modifierPoll = 0;
        this.windows = new Map();
        this.setupPrivacy();
        const directory = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'gnoblin']);
        GLib.mkdir_with_parents(directory, 0o700);
        this.path = Gio.File.new_for_path(GLib.getenv('GNOBLIN_COMPOSITOR_SOCKET') || `${directory}/compositor-v1.sock`);
        if (this.path.query_exists(null)) this.path.delete(null);
        this.service = new Gio.SocketService();
        this.service.add_address(Gio.UnixSocketAddress.new(this.path.get_path()),
            Gio.SocketType.STREAM, Gio.SocketProtocol.DEFAULT, null);
        this.service.connect('incoming', (_service, connection) => { this.accept(connection); return true; });
        this.service.start();
        this.accelerator = global.display.connect('accelerator-activated', (_display, action) => this.activate(action));
        this.capture = global.stage.connect('event', (_stage, event) => this.event(event));
        this.session = Main.sessionMode.connect('updated', () => {
            if (Main.sessionMode.isLocked) {
                this.caret = null;
                this.end('cancelled');
            }
        });
        this.map = global.window_manager.connect('map', (_manager, actor) => this.track(actor.meta_window));
        this.caret = null;
        this.focus = global.display.connect('notify::focus-window', () => {
            this.caret = null;
            this.publishWindows();
        });
        this.cursorLocation = Main.inputMethod.connect('cursor-location-changed', (_method, rect) => {
            const window = global.display.focus_window;
            const focus = Main.inputMethod.currentFocus;
            if (!window || !focus || Main.sessionMode.isLocked) return;
            const buffer = window.get_buffer_rect();
            // Mutter supplies stage coordinates. Store the offset so moving
            // the window does not leave the caret at its previous location.
            this.caret = {window, focus, x: rect.get_x() - buffer.x,
                y: rect.get_y() - buffer.y, width: rect.get_width(), height: rect.get_height()};
        });
        for (const actor of global.get_window_actors()) this.track(actor.meta_window);
    }

    accept(connection) {
        if (this.clients.size >= 8) { connection.close(null); return; }
        const client = {connection, cancel: new Gio.Cancellable(), buffer: new Uint8Array(), queue: [], queuedBytes: 0, writing: false,
            bindings: new Map(), decoder: new TextDecoder('utf-8', {fatal: true}), closed: false};
        this.clients.add(client);
        this.send(client, {event: 'hello', version: 1});
        this.read(client);
    }

    send(client, record) {
        if (client.closed) return;
        if (client.queue.length > 64) { this.close(client); return; }
        const bytes = new TextEncoder().encode(JSON.stringify(record) + '\n');
        if (client.queuedBytes + bytes.length > 4 * 1024 * 1024) { this.close(client); return; }
        client.queuedBytes += bytes.length;
        client.queue.push(bytes);
        this.write(client);
    }

    write(client) {
        if (client.closed || client.writing || !client.queue.length) return;
        client.writing = true;
        client.connection.get_output_stream().write_all_async(client.queue[0], GLib.PRIORITY_DEFAULT,
            client.cancel, (stream, result) => {
                try { stream.write_all_finish(result); }
                catch (error) {
                    if (!client.closed) console.warn(`gnoblin-compositor write: ${error.message}`);
                    this.close(client); return;
                }
                client.queuedBytes -= client.queue.shift().length;
                client.writing = false;
                this.write(client);
            });
    }

    read(client) {
        client.connection.get_input_stream().read_bytes_async(4096, GLib.PRIORITY_DEFAULT, client.cancel, (stream, result) => {
            try {
                const bytes = stream.read_bytes_finish(result).toArray();
                if (!bytes.length) { this.close(client); return; }
                if (client.buffer.length + bytes.length > 16384) throw new Error('input record too large');
                const buffer = new Uint8Array(client.buffer.length + bytes.length);
                buffer.set(client.buffer);
                buffer.set(bytes, client.buffer.length);
                client.buffer = buffer;
                let newline;
                while ((newline = client.buffer.indexOf(10)) >= 0) {
                    const record = JSON.parse(client.decoder.decode(client.buffer.subarray(0, newline)));
                    client.buffer = client.buffer.slice(newline + 1);
                    try { this.command(client, record); }
                    catch (error) { this.send(client, {event: 'error', id: record.id, message: error.message}); }
                }
                if (!client.closed) this.read(client);
            } catch (error) {
                if (!client.closed) console.warn(`gnoblin-compositor read: ${error.message}`);
                this.close(client);
            }
        });
    }

    command(client, record) {
        if (record.op === 'shortcut-input') {
            if (Main.sessionMode.isLocked || typeof record.name !== 'string' ||
                !['prepared', 'ready', 'closed'].includes(record.state))
                throw new Error('Invalid input handoff');
            const handoff = Main.componentManager?._allComponents?.gnoblinControl?._shortcutInput;
            if (record.state === 'prepared') handoff?.prepared(record.name);
            else if (record.state === 'ready') handoff?.complete(record.name);
            else handoff?.closed(record.name);
            return;
        }
        if (record.op === 'command') {
            if (typeof record.id !== 'string' || !/^[\w-]{1,64}$/.test(record.id)) throw new Error('invalid request ID');
            this.send(client, {event: 'reply', id: record.id, result: this.control(record)});
            return;
        }
        if (record.op === 'input-anchor') {
            if (Main.sessionMode.isLocked) throw new Error('Session is locked.');
            const window = global.display.focus_window;
            const [x, y] = global.get_pointer();
            const frame = window?.get_frame_rect();
            const buffer = window?.get_buffer_rect();
            const caret = this.caret?.window === window && Main.inputMethod.currentFocus
                && this.caret.focus === Main.inputMethod.currentFocus ? this.caret : null;
            this.send(client, {event: 'input-anchor', x, y,
                caret: caret ? {x: buffer.x + caret.x, y: buffer.y + caret.y,
                    width: caret.width, height: caret.height, source: 'caret'} : null,
                pid: window?.get_pid() || 0,
                window: window ? String(window.get_stable_sequence()) : '',
                buffer: buffer ? {x: buffer.x, y: buffer.y, width: buffer.width, height: buffer.height} : null,
                frame: frame ? {x: frame.x, y: frame.y, width: frame.width, height: frame.height} : null});
            return;
        }
        if (record.op === 'type-text') {
            const window = this.windows.get(record.window)?.window;
            if (Main.sessionMode.isLocked || !window || !this.eligible(window)
                || global.display.focus_window !== window)
                throw new Error('The original input window is no longer focused.');
            if (typeof record.text !== 'string' || !record.text.length || record.text.length > 64
                || /[\u0000-\u001f\u007f-\u009f]/.test(record.text))
                throw new Error('Invalid text insertion.');
            const modifiers = global.get_pointer()[2];
            if (modifiers & (Clutter.ModifierType.CONTROL_MASK | Clutter.ModifierType.MOD1_MASK
                | Clutter.ModifierType.MOD4_MASK | Clutter.ModifierType.SUPER_MASK))
                throw new Error('Release modifier keys before inserting an emoji.');
            if (window.get_client_type() === Meta.WindowClientType.X11) {
                this.clipboardPaste.paste(record.text, () => {
                    if (client.closed || Main.sessionMode.isLocked || global.display.focus_window !== window)
                        throw new Error('The original input window is no longer focused.');
                    if (global.get_pointer()[2] & (Clutter.ModifierType.SHIFT_MASK | Clutter.ModifierType.CONTROL_MASK
                        | Clutter.ModifierType.MOD1_MASK | Clutter.ModifierType.MOD4_MASK | Clutter.ModifierType.SUPER_MASK))
                        throw new Error('Release modifier keys before inserting an emoji.');
                }).then(() => this.send(client, {event: 'typed', window: record.window}),
                    error => this.send(client, {event: 'error', message: error.message}));
                return;
            }
            if (!Main.inputMethod.currentFocus)
                throw new Error('This app does not expose a text input. Focus its input field and try again.');
            // Commit the complete Unicode sequence through the native input
            // method, avoiding layout-dependent synthetic keycodes.
            Main.inputMethod.commit(record.text);
            this.send(client, {event: 'typed', window: record.window});
            return;
        }
        if (record.op === 'status') {
            this.send(client, {event: 'status', bindings: [...this.actions.values()].map(binding => binding.id),
                active: this.active?.id ?? null});
            return;
        }
        if (record.op === 'privacy') { client.trackPrivacy = true; this.publishPrivacy(client); return; }
        if (record.op === 'stop-sharing' || record.op === 'stop-recording') {
            const recording = record.op === 'stop-recording';
            for (const [handle, state] of this.remoteHandles)
                if (state.recording === recording) handle.stop();
            return;
        }
        if (record.op === 'windows') { client.trackWindows = true; this.publishWindows(client); return; }
        if (record.op === 'preview') {
            if (typeof record.window !== 'string' || !Number.isInteger(record.width) || !Number.isInteger(record.height)
                || record.width < 1 || record.width > 480 || record.height < 1 || record.height > 320)
                throw new Error('invalid preview request');
            if (client.preview || client.previewBusy) throw new Error('preview already pending');
            // Give keyboard input precedence over thumbnail readback.
            client.preview = GLib.timeout_add(GLib.PRIORITY_LOW, 32, () => {
                client.preview = 0;
                this.preview(client, record);
                return GLib.SOURCE_REMOVE;
            });
            return;
        }
        if (record.op === 'activate') {
            const entry = this.windows.get(record.window);
            if (!entry || !this.eligible(entry.window)) throw new Error('window no longer available');
            if (this.active?.client === client) this.end('cancelled');
            Main.activateWindow(entry.window, global.get_current_time());
            return;
        }
        if (record.op === 'end') {
            if (this.active?.client === client) this.end('cancelled');
            return;
        }
        if (record.op === 'clear') { this.clear(client); return; }
        if (record.op !== 'bind' || typeof record.id !== 'string' || !/^[\w-]{1,64}$/.test(record.id) ||
            typeof record.accelerator !== 'string' || record.accelerator.length > 128 ||
            !Number.isInteger(record.hold) || ![0, Clutter.ModifierType.MOD1_MASK, Clutter.ModifierType.SUPER_MASK,
                Clutter.ModifierType.CONTROL_MASK].includes(record.hold) ||
            (record.modal !== undefined && typeof record.modal !== 'boolean') ||
            client.bindings.size >= 32 || client.bindings.has(record.id))
            throw new Error('invalid shortcut registration');
        const action = global.display.grab_accelerator(record.accelerator, Meta.KeyBindingFlags.NONE);
        if (action === Meta.KeyBindingAction.NONE) throw new Error(`shortcut already claimed: ${record.accelerator}`);
        const binding = {client, id: record.id, hold: record.hold, modal: record.modal !== false, action};
        client.bindings.set(record.id, binding);
        this.actions.set(action, binding);
        Main.wm.allowKeybinding(Meta.external_binding_name_for_action(action), Shell.ActionMode.NORMAL);
        this.send(client, {event: 'bound', id: record.id});
    }

    activate(action) {
        const binding = this.actions.get(action);
        if (!binding) return;
        if (this.active && this.active.client !== binding.client) this.end('cancelled');
        const first = !this.active;
        if (first && binding.hold) {
            this.active = {...binding, focusWindow: global.display.focus_window};
            if (binding.modal) {
                // Input-only sessions need the stage grab so they receive
                // navigation and release before their surface maps.
                this.grab = Main.pushModal(global.stage, {actionMode: Shell.ActionMode.POPUP});
                if (!(this.grab.get_seat_state() & Clutter.GrabState.KEYBOARD)) {
                    this.end('cancelled');
                    return;
                }
            }
            this.timeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 10000, () => {
                this.timeout = 0;
                this.end('cancelled');
                return GLib.SOURCE_REMOVE;
            });
            if (!binding.modal) this.startPassiveModifierPoll();
        }
        this.send(binding.client, {event: 'activated', id: binding.id, first,
            modifiers: global.get_pointer()[2], time: global.get_current_time()});
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
                this.end('released');
                return GLib.SOURCE_REMOVE;
            }
            return GLib.SOURCE_CONTINUE;
        });
    }

    checkModifiers() {
        if (this.modifierCheck) return;
        this.modifierCheck = GLib.idle_add(GLib.PRIORITY_HIGH_IDLE, () => {
            this.modifierCheck = 0;
            if (this.active && !this.modifiersHeld(global.get_pointer()[2], this.active.hold)) this.end('released');
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
                if (event.get_key_symbol() === Clutter.KEY_Super_L ||
                    event.get_key_symbol() === Clutter.KEY_Super_R)
                    return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        }
        if (type === Clutter.EventType.BUTTON_PRESS) {
            const [x, y] = event.get_coords();
            this.send(this.active.client, {event: 'pointer', x, y, button: event.get_button()});
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
            this.end('released');
            return Clutter.EVENT_PROPAGATE;
        }
        if (type === Clutter.EventType.KEY_PRESS) {
            const action = global.display.get_keybinding_action(event.get_key_code(), event.get_state());
            if (this.actions.has(action)) this.activate(action);
            else
                this.send(this.active.client, {event: 'key', key: event.get_key_symbol(), modifiers: event.get_state()});
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
            if (!Main.sessionMode.isLocked && active.focusWindow?.get_compositor_private()
                && !active.focusWindow.minimized)
                active.focusWindow.focus(global.get_current_time());
        }
        this.send(active.client, {event: reason});
    }

    clear(client) {
        if (this.active?.client === client) this.end('cancelled');
        for (const binding of client.bindings.values()) {
            global.display.ungrab_accelerator(binding.action);
            Main.wm.allowKeybinding(Meta.external_binding_name_for_action(binding.action), Shell.ActionMode.NONE);
            this.actions.delete(binding.action);
        }
        client.bindings.clear();
    }

    eligible(window) {
        return window && !window.skip_taskbar && !window.is_override_redirect();
    }

    setupPrivacy() {
        // The active handles and their start times survive script reloads. Only
        // each handle's stopped callback persists, until that session ends.
        this.remoteHandles = global.__gnoblinRemoteAccessHandles ??= new Map();
        global.__gnoblinPublishPrivacy = () => this.publishPrivacy();
        this.remoteController = global.backend.get_remote_access_controller();
        this.remoteSignal = this.remoteController?.connect('new-handle', (_controller, handle) => this.trackRemote(handle));
        const panel = Main.panel?.statusArea;
        for (const handles of [panel?.screenSharing?._handles, panel?.quickSettings?._remoteAccess?._handles])
            for (const handle of handles ?? []) this.trackRemote(handle);
        this.ownsCameraMonitor = !panel?.quickSettings?._camera?._cameraMonitor;
        this.cameraMonitor = panel?.quickSettings?._camera?._cameraMonitor ?? new Shell.CameraMonitor();
        this.cameraSignal = this.cameraMonitor.connect('notify::cameras-in-use', () => this.publishPrivacy());
    }

    trackRemote(handle) {
        if (this.remoteHandles.has(handle)) return;
        const state = {recording: Boolean(handle.is_recording ?? handle.isRecording), started: GLib.get_monotonic_time()};
        this.remoteHandles.set(handle, state);
        state.signal = handle.connect('stopped', () => {
            global.__gnoblinRemoteAccessHandles.delete(handle);
            handle.disconnect(state.signal);
            global.__gnoblinPublishPrivacy?.();
        });
        this.publishPrivacy();
    }

    publishPrivacy(client = null) {
        const states = [...this.remoteHandles.values()];
        const recordings = states.filter(state => state.recording);
        const started = recordings.length ? Math.min(...recordings.map(state => state.started)) : 0;
        const record = {event: 'privacy', screenSharing: states.some(state => !state.recording),
            recording: recordings.length > 0, recordingCount: recordings.length,
            recordingElapsed: started ? Math.floor((GLib.get_monotonic_time() - started) / 1000000) : 0,
            cameraInUse: Boolean(this.cameraMonitor?.cameras_in_use)};
        const clients = client ? [client] : [...this.clients].filter(peer => peer.trackPrivacy);
        for (const peer of clients) this.send(peer, record);
    }

    async preview(client, request) {
        client.previewBusy = true;
        const stream = Gio.MemoryOutputStream.new_resizable();
        try {
            if (Main.sessionMode.isLocked) throw new Error('session locked');
            const window = this.windows.get(request.window)?.window;
            if (!this.eligible(window)) throw new Error('window no longer available');
            // Keep full-size pixels on the GPU. Only the small render target is
            // read back; Shell's PNG encoder runs asynchronously on a worker.
            const actor = window.get_compositor_private();
            // Occlusion can make actor painting transparent, including when
            // the chooser covers a mapped window. Sample the backing buffer
            // directly where possible; it also survives minimisation.
            const backing = actor?.get_texture()?.get_texture();
            const source = backing && (!backing.is_simple || backing.is_simple())
                ? (backing.get_plane ? backing.get_plane(0) : backing)
                : actor?.paint_to_content(null)?.get_texture();
            if (!source) throw new Error('window has no image');
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
            pipeline.set_blend('RGBA = ADD (SRC_COLOR, 0)');
            framebuffer.draw_rectangle(pipeline, 0, 0, width, height);
            // composite_to_stream reads a subtexture. Flush this render target
            // explicitly; the subtexture does not own its pending draw journal.
            framebuffer.flush();
            const pixbuf = await Shell.Screenshot.composite_to_stream(texture, 0, 0, width, height, 1, null, 0, 0, 1, stream);
            if (pixbuf.get_has_alpha()) {
                const pixels = pixbuf.get_pixels();
                const stride = pixbuf.get_rowstride();
                const channels = pixbuf.get_n_channels();
                let visible = false;
                for (let y = 0; y < height && !visible; y++)
                    for (let x = 0; x < width; x++)
                        if (pixels[y * stride + x * channels + channels - 1]) { visible = true; break; }
                if (!visible) throw new Error('window image buffer unavailable');
            }
            stream.close(null);
            if (client.closed || Main.sessionMode.isLocked || !this.windows.has(request.window)) return;
            const bytes = stream.steal_as_bytes().toArray();
            this.send(client, {event: 'preview', window: request.window, width, height,
                source: `data:image/png;base64,${GLib.base64_encode(bytes)}`});
        } catch (error) {
            this.send(client, {event: 'preview', window: request.window, source: '', message: error.message});
        } finally {
            stream.close(null);
            client.previewBusy = false;
        }
    }

    track(window) {
        if (!window) return;
        const id = String(window.get_stable_sequence());
        if (this.windows.has(id)) return;
        const signals = ['notify::title', 'notify::minimized', 'notify::skip-taskbar'].map(signal =>
            window.connect(signal, () => this.publishWindows()));
        signals.push(window.connect('unmanaged', () => {
            this.windows.delete(id);
            for (const signal of signals) window.disconnect(signal);
            this.publishWindows();
        }));
        this.windows.set(id, {window, signals});
        this.publishWindows();
    }

    windowRecords() {
        const tracker = Shell.WindowTracker.get_default();
        return [...this.windows].filter(([_id, entry]) => this.eligible(entry.window)).map(([id, {window}]) => {
            const monitor = Main.layoutManager.monitors[window.get_monitor()];
            const frame = window.get_frame_rect();
            const app = tracker.get_window_app(window);
            return {id, title: window.title || '', appId: app && !app.is_window_backed() ? app.get_id() : window.get_wm_class() || '',
                focused: global.display.focus_window === window, minimized: window.minimized,
                workspace: window.get_workspace()?.index() + 1 || null, monitorIndex: window.get_monitor(),
                maximized: window.get_maximize_flags() === Meta.MaximizeFlags.BOTH, fullscreen: window.is_fullscreen(),
                geometry: {x: frame.x, y: frame.y, width: frame.width, height: frame.height},
                lastUserTime: window.get_user_time(), parent: window.get_transient_for()
                    ? String(window.get_transient_for().get_stable_sequence()) : null,
                monitor: monitor ? {x: monitor.x, y: monitor.y} : null};
        });
    }

    control(record) {
        if (record.command === 'windows') return {windows: this.windowRecords()};
        const manager = global.workspace_manager;
        if (record.command === 'workspaces') return {workspaces: Array.from({length: manager.n_workspaces}, (_, index) => ({
            id: index + 1, active: index === manager.get_active_workspace_index(),
            windows: manager.get_workspace_by_index(index).list_windows().filter(window => this.eligible(window)).length,
        }))};
        if (record.command === 'monitors') return {monitors: Main.layoutManager.monitors.map(monitor => ({
            id: monitor.index, x: monitor.x, y: monitor.y, width: monitor.width, height: monitor.height,
            primary: monitor.index === Main.layoutManager.primaryIndex, scale: global.display.get_monitor_scale(monitor.index),
        }))};
        if (Main.sessionMode.isLocked) throw new Error('window management is unavailable while the session is locked');
        const workspace = () => {
            if (!Number.isInteger(record.workspace) || record.workspace < 1 || record.workspace > manager.n_workspaces)
                throw new Error('workspace not found; list workspaces first');
            return manager.get_workspace_by_index(record.workspace - 1);
        };
        if (record.command === 'workspace-switch') {
            workspace().activate(global.get_current_time());
            return {ok: true, pending: true, workspace: record.workspace};
        }
        if (record.command !== 'window') throw new Error('unknown compositor command');
        const actions = ['focus', 'close', 'minimize', 'restore', 'maximize', 'unmaximize', 'fullscreen', 'unfullscreen', 'move', 'resize', 'workspace', 'monitor'];
        if (!actions.includes(record.action)) throw new Error('unknown window action');
        const window = record.window === 'active' ? global.display.focus_window : this.windows.get(record.window)?.window;
        if (!window || !this.eligible(window)) throw new Error('window no longer available; list windows first');
        const requireCapability = (allowed, message) => { if (!allowed) throw new Error(message); };
        switch (record.action) {
        case 'focus': Main.activateWindow(window, global.get_current_time()); break;
        case 'close': requireCapability(window.can_close(), 'window cannot be closed'); window.delete(global.get_current_time()); break;
        case 'minimize': requireCapability(window.can_minimize(), 'window cannot be minimized'); window.minimize(); break;
        case 'restore': window.unminimize(); break;
        case 'maximize': requireCapability(window.can_maximize(), 'window cannot be maximized'); window.maximize(Meta.MaximizeFlags.BOTH); break;
        case 'unmaximize': window.unmaximize(Meta.MaximizeFlags.BOTH); break;
        case 'fullscreen': window.make_fullscreen(); break;
        case 'unfullscreen': window.unmake_fullscreen(); break;
        case 'move':
            requireCapability(window.allows_move() && !window.is_fullscreen() && !window.get_maximize_flags(), 'window cannot move; unmaximize or leave fullscreen first');
            if (![record.x, record.y].every(value => Number.isInteger(value) && Math.abs(value) <= 100000)) throw new Error('invalid window position');
            window.move_frame(true, record.x, record.y);
            break;
        case 'resize': {
            requireCapability(window.allows_resize() && !window.is_fullscreen() && !window.get_maximize_flags(), 'window cannot resize; unmaximize or leave fullscreen first');
            if (![record.width, record.height].every(value => Number.isInteger(value) && value >= 1 && value <= 32768)) throw new Error('invalid window size');
            const frame = window.get_frame_rect();
            window.move_resize_frame(true, frame.x, frame.y, record.width, record.height);
            break;
        }
        case 'workspace': window.change_workspace(workspace()); break;
        case 'monitor':
            if (!Number.isInteger(record.monitor) || !Main.layoutManager.monitors.some(monitor => monitor.index === record.monitor)) throw new Error('monitor not found');
            window.move_to_monitor(record.monitor);
            break;
        }
        return {ok: true, pending: true, window: String(window.get_stable_sequence()), action: record.action};
    }

    publishWindows(client = null) {
        const clients = client ? [client] : [...this.clients].filter(peer => peer.trackWindows);
        if (!clients.length) return;
        const windows = this.windowRecords();
        for (const peer of clients) this.send(peer, {event: 'windows', windows});
    }

    close(client) {
        if (client.closed) return;
        client.closed = true;
        if (client.preview) GLib.source_remove(client.preview);
        client.preview = 0;
        this.clear(client);
        client.cancel.cancel();
        try { client.connection.close(null); } catch { /* Connection already closed. */ }
        this.clients.delete(client);
    }

    destroy() {
        this.clipboardPaste.destroy();
        this.end('cancelled');
        global.__gnoblinPublishPrivacy = null;
        if (this.remoteSignal) this.remoteController.disconnect(this.remoteSignal);
        this.cameraMonitor.disconnect(this.cameraSignal);
        if (this.ownsCameraMonitor) this.cameraMonitor.run_dispose();
        for (const client of this.clients) this.close(client);
        global.display.disconnect(this.accelerator);
        global.stage.disconnect(this.capture);
        Main.sessionMode.disconnect(this.session);
        global.window_manager.disconnect(this.map);
        global.display.disconnect(this.focus);
        Main.inputMethod.disconnect(this.cursorLocation);
        for (const {window, signals} of this.windows.values()) for (const signal of signals) window.disconnect(signal);
        this.windows.clear();
        this.service.stop();
        this.service.close();
        this.path.delete(null);
    }
}

export default function enable(api) {
    const bridge = new CompositorBridge();
    api._disposers.push(() => bridge.destroy());
}
