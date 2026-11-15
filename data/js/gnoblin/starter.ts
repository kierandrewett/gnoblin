// The stock default: what the shell runs when there is no init.ts. It is ordinary imperative code against the objects of
// the specification, in the style of the owner's reference init.js, so a user can read it, copy it
// (`gnoblinctl config init` writes the same statements as a file, data/js/starter/init.ts) or call it from their own file
// to keep the stock workspaces and keys while adding their own:
//
//     import { installDefaults } from "gnoblin/starter";
//     installDefaults(gnoblin);
//
// There is no merge: an init.ts that does not call this function has none of it.
//
// The statements of `installDefaults` and of data/js/starter/init.ts are one text. A test compares them, so edit both.

import { Workspace, startDecorations, startNotRespondingAlert, startWindowMenu } from "gnoblin/host";
import type { gnoblin as Gnoblin } from "gnoblin/host";
import { WorkspaceSlideBinding } from "gnoblin/gestures";
import { FloatingLayout } from "gnoblin/layouts/floating";
import { StripLayout } from "gnoblin/layouts/strip";
import { openLauncher } from "gnoblin/launcher";

/** What the stock default needs of the `gnoblin` global. */
type Host = typeof Gnoblin;

/**
 * Declares the stock default on `gnoblin`: four floating workspaces, titlebars, the window menu, the not-responding
 * alert, workspace keys, strip focus keys, the three-finger workspace swipe, the launcher key and the media keys.
 */
export function installDefaults(gnoblin: Host): void {
    // Four floating workspaces. The first is shown until a key shows another.
    for (const id of ["1", "2", "3", "4"]) {
        gnoblin.workspaces.add(new Workspace({ id, name: `Workspace ${id}`, layout: new FloatingLayout() }));
    }

    // A titlebar on every window, the menu the compositor asks for, and the dialog for a window that stops answering.
    startDecorations();
    startWindowMenu();
    startNotRespondingAlert();

    const input = gnoblin.input.primary;

    // Super+1 .. Super+9 show the workspace at that position on the display the pointer is on. A key past the last
    // workspace does nothing.
    for (let position = 1; position <= 9; position++) {
        input.bind(`Super+${position}`, () => {
            const target = gnoblin.workspaces.at(position - 1);
            if (target !== undefined) {
                gnoblin.workspaces.show(target, { duration: 250 });
            }
        });
    }

    // Super+Left and Super+Right move the focus between the columns of a StripLayout, and do nothing on other layouts.
    const moveColumn = (side: "focusLeft" | "focusRight") => {
        const shown = (input.activeDisplay as { viewport: { content: Workspace | null } } | null)?.viewport.content;
        if (shown?.layout instanceof StripLayout) {
            shown.layout[side](shown);
        }
    };
    input.bind("Super+Left", () => moveColumn("focusLeft"));
    input.bind("Super+Right", () => moveColumn("focusRight"));

    // Three fingers sliding sideways slide between workspaces.
    input.bind("Swipe3", new WorkspaceSlideBinding());

    // Volume, microphone and player keys.
    const run = (command: string[]) => {
        gnoblin.spawn(command);
    };
    input.bind("XF86AudioRaiseVolume", () => run(["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"]));
    input.bind("XF86AudioLowerVolume", () => run(["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"]));
    input.bind("XF86AudioMute", () => run(["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]));
    input.bind("XF86AudioMicMute", () => run(["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"]));
    input.bind("XF86AudioPlay", () => run(["playerctl", "play-pause"]));
    input.bind("XF86AudioNext", () => run(["playerctl", "next"]));
    input.bind("XF86AudioPrev", () => run(["playerctl", "previous"]));

    // Super+space opens the launcher: a command entry in the middle of the screen.
    input.bind("Super+space", () => {
        openLauncher();
    });
}
