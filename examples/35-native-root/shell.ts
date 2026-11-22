// Example 35 in TypeScript: the members of the `gnoblin` root object of libgnoblin-core, through the classes generated from its catalog.
//
//     gnoblin ./shell.ts
//
// Does what the Rust shell of the same example (`35-native-root`) does and logs the same lines, which start with `root:`: reads the
// build identity with `version()`, sets a timer with `timers.after` and cancels another, lists the windows and outputs with `find`, and when a
// window appears reads `motion(200)` and `compositor.userShaders`. `find` also refuses a kind the core does not list, and a field a window has
// no filter for, and says what works.
import { gnoblin } from "gnoblin-core/generated/objects";

const say = (text: string) => console.log(`root: ${text}`);

const version = gnoblin.version();
say(`gnoblin ${version.gnoblin} speaks api ${version.api}`);

const started = gnoblin.timers.now();
gnoblin.timers.after(40, () => {
    say(`the timer fired after its time: ${gnoblin.timers.now() - started >= 39}`);
});
const never = gnoblin.timers.after(10, () => say("a cancelled timer ran"));
never.cancel();
say(`a cancelled timer is pending: ${never.pending}`);

try {
    gnoblin.find("Workspace");
} catch (error) {
    say(`an unknown kind is refused: ${(error as Error).message}`);
}
try {
    gnoblin.find("Window", { noSuchField: 1 } as never);
} catch (error) {
    say(`an unknown field is refused: ${(error as Error).message}`);
}

gnoblin.onClientCreated((client) => {
    const windows = gnoblin.find("Window").length;
    const foot = gnoblin.find("Window", { appId: "foot" }).length;
    const titled = gnoblin.find("Window", (found) => found.title.startsWith("Term")).length;
    const outputs = gnoblin.find("Output").length;
    say(
        `created ${client.title}: windows = ${windows}, appId foot = ${foot}, title starts with Term = ${titled}, outputs listed = ${outputs > 0}, ` +
            `motion(200) = ${gnoblin.motion(200)}, userShaders = ${gnoblin.compositor.userShaders}`,
    );
});
say("running");
