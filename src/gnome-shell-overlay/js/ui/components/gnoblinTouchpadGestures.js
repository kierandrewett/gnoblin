import Gio from "gi://Gio";
import Meta from "gi://Meta";
import Shell from "gi://Shell";

import * as Main from "../main.js";
import { TouchpadGestureRouter as GestureRouter } from "./gnoblinTouchpadGestureCore.js";

function gestureContexts() {
    if (Main.keyboard?._keyboard?._emojiSelection?.visible) return ["emoji-picker"];
    if (Main.actionMode & Shell.ActionMode.UNLOCK_SCREEN) return ["unlock-screen"];
    return Main.actionMode & Shell.ActionMode.NORMAL ? ["normal"] : [];
}

export class TouchpadGestureRouter {
    constructor() {
        this._router = new GestureRouter({
            getContexts: gestureContexts,
            runGesture: (gesture) => this._run(gesture),
        });
    }

    configure(gestures) {
        this._router.configure(gestures);
    }

    destroy() {
        this._router.destroy();
        this._router = null;
    }

    handle(payload) {
        return this._router?.handle(payload) ?? false;
    }

    _run(gesture) {
        if (gesture.command) {
            try {
                const process = Gio.Subprocess.new(gesture.command, Gio.SubprocessFlags.NONE);
                process.wait_async(null, () => {});
            } catch (error) {
                console.warn(`gnoblin touchpad gesture ${gesture.name}: ${error.message}`);
            }
            return;
        }

        const window = global.display.focus_window;
        switch (gesture.action) {
            case "workspace.next":
            case "workspace.previous": {
                const manager = global.workspace_manager;
                const delta = gesture.action === "workspace.next" ? 1 : -1;
                const index = manager.get_active_workspace_index() + delta;
                if (index >= 0 && index < manager.get_n_workspaces())
                    manager.get_workspace_by_index(index).activate(global.get_current_time());
                break;
            }
            case "window.close":
                window?.delete(global.get_current_time());
                break;
            case "window.minimize":
                window?.minimize();
                break;
            case "window.toggle-maximize":
                if (window) {
                    if (window.get_maximized() === Meta.MaximizeFlags.BOTH) window.unmaximize(Meta.MaximizeFlags.BOTH);
                    else window.maximize(Meta.MaximizeFlags.BOTH);
                }
                break;
            default:
                console.warn(`gnoblin touchpad gesture ${gesture.name}: unsupported action ${gesture.action}`);
        }
    }
}
