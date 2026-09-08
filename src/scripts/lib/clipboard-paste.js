import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

function readRecord(stream, cancel) {
    return new Promise((resolve, reject) => stream.read_line_async(GLib.PRIORITY_DEFAULT, cancel, (input, result) => {
        try {
            const [line] = input.read_line_finish_utf8(result);
            if (line === null) throw new Error('The clipboard helper stopped before completing the paste.');
            const record = JSON.parse(line);
            if (record.event === 'error') throw new Error(record.message);
            resolve(record.event);
        } catch (error) { reject(error); }
    }));
}

export class ClipboardPaste {
    constructor(seat) {
        this.seat = seat;
        this.cancel = null;
    }

    async paste(text, checkFocus) {
        if (this.cancel) throw new Error('A paste is already in progress.');
        const cancel = new Gio.Cancellable();
        this.cancel = cancel;
        let child, keyboard;
        const timeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 4500, () => {
            cancel.cancel();
            return GLib.SOURCE_CONTINUE;
        });
        try {
            checkFocus();
            const helper = Gio.File.new_for_uri(import.meta.url).get_parent().get_child('clipboard-paste.py').get_path();
            const launcher = new Gio.SubprocessLauncher({flags: Gio.SubprocessFlags.STDIN_PIPE | Gio.SubprocessFlags.STDOUT_PIPE});
            launcher.setenv('GDK_BACKEND', 'x11', true);
            child = launcher.spawnv(['python3', helper]);
            child.get_stdin_pipe().write_all(new TextEncoder().encode(JSON.stringify(text) + '\n'), null);
            const input = new Gio.DataInputStream({base_stream: child.get_stdout_pipe()});
            if (await readRecord(input, cancel) !== 'ready') throw new Error('Clipboard preparation failed.');
            await new Promise(resolve => GLib.timeout_add(GLib.PRIORITY_DEFAULT, 120, () => {
                resolve(); return GLib.SOURCE_REMOVE;
            }));
            child.get_stdin_pipe().write_all(new TextEncoder().encode('CHECK\n'), null);
            if (await readRecord(input, cancel) !== 'armed') throw new Error('Clipboard changed before paste.');
            checkFocus();
            keyboard = this.seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
            const key = (value, state) => keyboard.notify_keyval(GLib.get_monotonic_time(), value, state);
            key(Clutter.KEY_Control_L, Clutter.KeyState.PRESSED);
            key(Clutter.KEY_v, Clutter.KeyState.PRESSED);
            key(Clutter.KEY_v, Clutter.KeyState.RELEASED);
            key(Clutter.KEY_Control_L, Clutter.KeyState.RELEASED);
            child.get_stdin_pipe().write_all(new TextEncoder().encode('PASTE\n'), null);
            if (await readRecord(input, cancel) !== 'done') throw new Error('Clipboard restoration failed.');
        } finally {
            // EOF also tells the helper to restore after focus loss or reload.
            try { child?.get_stdin_pipe().close(null); } catch (_) { /* Already closed. */ }
            keyboard?.run_dispose();
            GLib.source_remove(timeout);
            this.cancel = null;
        }
    }

    destroy() { this.cancel?.cancel(); }
}
