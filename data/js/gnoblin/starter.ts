// The stock shell setup used when no `init.ts` is present.
// Call `installDefaults` from a custom `init.ts` to keep these defaults and add your own settings.
// An `init.ts` that omits this call uses only the settings it declares.
// `data/js/starter/init.ts` mirrors these statements; a test checks that they stay in sync.

import type { Gnoblin } from "gnoblin/host/gnoblin";
import { startDecorations, startNotRespondingAlert, startWindowMenu } from "gnoblin/host/services";
import { Workspace } from "gnoblin/host/workspace";
import { WorkspaceSlideBinding } from "gnoblin/gestures";
import { FloatingLayout } from "gnoblin/layouts/floating";
import { StripLayout } from "gnoblin/layouts/strip";
import { openLauncher } from "gnoblin/launcher";

/**
 * Installs the stock workspaces, window decorations, input bindings and launcher on `gnoblin`.
 *
 * @example
 * ```ts
 * import { installDefaults } from "gnoblin/starter";
 * installDefaults(gnoblin);
 * ```
 */
export function installDefaults(gnoblin: Gnoblin): void {
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
