import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import Cogl from 'gi://Cogl';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

Gio._promisify(Shell.Screenshot, 'composite_to_stream');

// Generic shortcut sessions and window management. Clients own presentation,
// ordering and action semantics.
// Newline-delimited JSON stays on one persistent, user-private Unix socket.
class CompositorBridge {
    constructor() {
        this.clients = new Set();
        this.actions = new Map();
        this.active = null;
        this.grab = null;
        this.timeout = 0;
        this.modifierCheck = 0;
        this.windows = new Map();
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
            if (Main.sessionMode.isLocked) this.end('cancelled');
        });
        this.map = global.window_manager.connect('map', (_manager, actor) => this.track(actor.meta_window));
        this.focus = global.display.connect('notify::focus-window', () => this.publishWindows());
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
        if (record.op === 'status') {
            this.send(client, {event: 'status', bindings: [...this.actions.values()].map(binding => binding.id),
                active: this.active?.id ?? null});
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
                Clutter.ModifierType.CONTROL_MASK].includes(record.hold) || client.bindings.size >= 32 || client.bindings.has(record.id))
            throw new Error('invalid shortcut registration');
        const action = global.display.grab_accelerator(record.accelerator, Meta.KeyBindingFlags.NONE);
        if (action === Meta.KeyBindingAction.NONE) throw new Error(`shortcut already claimed: ${record.accelerator}`);
        const binding = {client, id: record.id, hold: record.hold, action};
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
            this.active = binding;
            // Input only: the client supplies its own layer-shell surface.
            // Grabbing the stage catches release even before that surface maps.
            this.grab = Main.pushModal(global.stage, {actionMode: Shell.ActionMode.POPUP});
            if (!(this.grab.get_seat_state() & Clutter.GrabState.KEYBOARD)) {
                this.end('cancelled');
                return;
            }
            this.timeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 10000, () => {
                this.timeout = 0;
                this.end('cancelled');
                return GLib.SOURCE_REMOVE;
            });
        }
        this.send(binding.client, {event: 'activated', id: binding.id, first,
            modifiers: global.get_pointer()[2], time: global.get_current_time()});
        // A release may precede the client receiving activation. Send both
        // records in order instead of waiting for the client to map a surface.
        if (this.active && !(global.get_pointer()[2] & this.active.hold)) this.end('released');
    }

    event(event) {
        if (!this.active) return Clutter.EVENT_PROPAGATE;
        const type = event.type();
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
            if (!this.modifierCheck) this.modifierCheck = GLib.idle_add(GLib.PRIORITY_HIGH_IDLE, () => {
                this.modifierCheck = 0;
                if (this.active && !(global.get_pointer()[2] & this.active.hold)) this.end('released');
                return GLib.SOURCE_REMOVE;
            });
            return Clutter.EVENT_PROPAGATE;
        }
        if (!(global.get_pointer()[2] & this.active.hold)) {
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
        if (this.timeout) GLib.source_remove(this.timeout);
        this.timeout = 0;
        if (!this.active) return;
        const active = this.active;
        this.active = null;
        if (this.grab) {
            const grab = this.grab;
            this.grab = null;
            Main.popModal(grab);
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

    publishWindows(client = null) {
        const clients = client ? [client] : [...this.clients].filter(peer => peer.trackWindows);
        if (!clients.length) return;
        const tracker = Shell.WindowTracker.get_default();
        const windows = [...this.windows].filter(([_id, entry]) => this.eligible(entry.window)).map(([id, {window}]) => {
            const monitor = Main.layoutManager.monitors[window.get_monitor()];
            return {id, title: window.title || '', appId: tracker.get_window_app(window)?.get_id() || window.get_wm_class() || '',
                focused: global.display.focus_window === window, minimized: window.minimized,
                lastUserTime: window.get_user_time(), parent: window.get_transient_for()
                    ? String(window.get_transient_for().get_stable_sequence()) : null,
                monitor: monitor ? {x: monitor.x, y: monitor.y} : null};
        });
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
        this.end('cancelled');
        for (const client of this.clients) this.close(client);
        global.display.disconnect(this.accelerator);
        global.stage.disconnect(this.capture);
        Main.sessionMode.disconnect(this.session);
        global.window_manager.disconnect(this.map);
        global.display.disconnect(this.focus);
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
