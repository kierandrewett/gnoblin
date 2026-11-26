// What the types of a subscription must refuse. Each `// expect:` line names the error the line after it must
// produce, and scripts/js-types-check.sh fails when a line has a different one, none, or when another line has any.
import { gnoblin } from "gnoblin-core";
import type { Client } from "gnoblin-core";
import "gnoblin-std";

// A title is text.
// expect: TS2345
gnoblin.clients.on("titleChanged", (_client: Client, title: number) => console.log(title));

// A collection of workspaces has no window events.
// expect: TS2345
gnoblin.workspaces.on("titleChanged", () => {});

// There is no such event of a window.
// expect: TS2339
gnoblin.onClientFlying(() => {});

// A window event is not an output event. The compiler may offer the nearest name instead.
// expect: TS2339|TS2551
gnoblin.onOutputTitleChanged(() => {});

// A signal's arguments are what the event carries: a bell has a count, and no text.
// expect: TS2345
gnoblin.session.on("bell", (_session, count: string) => console.log(count));
