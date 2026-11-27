// Example 27 in TypeScript: the window frame of libgnoblin-std, through the classes generated from its catalog.
//
//     gnoblin ./shell.ts
//
// The decorations give every window a frame and call `frame` to fill it. A window with a server-drawn frame gets the stock titlebar; the
// window titled `close me` is closed the way its close button closes it. Every line starts with `frame:`, as the Rust shell of the same
// example spells it.
import { Decorations, Titlebar } from "gnoblin-std";
import type {} from "gnoblin-std";

const decorations = Decorations.get();
decorations.frame = (client, frame) => {
    if (!frame.server) {
        console.log(`frame: ${client.appId} draws its own frame`);
        return true;
    }
    Titlebar.install(frame);
    console.log(`frame: ${client.appId} has a titlebar and its top edge takes ${frame.top.thickness}`);
    if (client.title === "close me") {
        console.log(`frame: closing ${client.appId} as its close button does`);
        frame.actions.close();
    }
    return true;
};
decorations.start();
console.log("frame: running");
