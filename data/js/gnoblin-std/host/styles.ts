// Hands the library the styles a script holds. What a style becomes on a window node (effects, opacity, corner radius) and what nothing
// draws is the library's: it applies them in its turn and in a transition's commit (`gnoblin-std/style/native`). The library keeps what
// it did not draw as notes of the client; the shell says each once, as a notice of the frame, when the styles moved.

import type { Client } from "gnoblin-core/host/client";
import { ctx } from "gnoblin-core/host/context";
import { say } from "gnoblin-core/host/dispatch";
import { stateOf } from "gnoblin-core/host/node_state";
import { everyStyle, pushStyles } from "gnoblin-std/host/style_scopes";
import { Style as NativeStyle } from "gnoblin-std/generated/classes";
import { notesOf } from "gnoblin-std/style/native";
import { syncStyleLists } from "gnoblin-std/style/lists";
import { styleEpoch } from "gnoblin-std/style/state";

/** What the notes were last asked at, and how many frames more to ask: the library applies a style in the turn after the frame that gave it. */
const noticed = { epoch: -1, clients: -1, frames: 0 };

/**
 * Starts this generation's styles from nothing in the library. A reload that stays in one core generation leaves the library's styles
 * and what it drew on the windows, so the generation clears them first (its own styles are made after); a library that starts empty
 * has nothing to clear. The library is then the owner of the styles, and its first pass takes what is left off the windows.
 */
export function installStyles(): void {
    NativeStyle.reset();
}

/** Checks the arrays a script changed in place, gives the library the styles that moved, and says what it did not draw. */
export function applyStyles(): void {
    for (const style of everyStyle()) {
        for (const error of syncStyleLists(style)) {
            ctx.reports.push({ site: "gnoblin.style", error });
        }
    }
    pushStyles();
    const epoch = styleEpoch();
    const clients = ctx.clientRoot.list.length;
    if (epoch !== noticed.epoch || clients !== noticed.clients) {
        noticed.epoch = epoch;
        noticed.clients = clients;
        noticed.frames = 3;
    }
    if (noticed.frames > 0) {
        noticed.frames -= 1;
        for (const client of ctx.clientRoot.list as Client[]) {
            if (stateOf(client).alive) {
                for (const note of notesOf(stateOf(client).node)) {
                    if (note.startsWith("not applied:")) {
                        say(note);
                    }
                }
            }
        }
    }
}
