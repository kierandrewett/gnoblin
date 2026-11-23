// Example 39 in TypeScript: why a call failed, and which call it was.
//
//     gnoblin ./shell.ts
//
// Does what the Rust shell of the same example (`39-native-errors`) does and logs the same lines, which start with `errors:`. A call the
// library refuses throws a `GnoblinError` with a `code` (`already_exists`, `not_found`) and a `target`, the object and member of the call
// (`Client#7.close`). A bad argument is not one: it throws a `TypeError` and has no code.
import { Gnoblin, GnoblinError, InputContext } from "gnoblin-core/generated/objects";

const say = (text: string) => console.log(`errors: ${text}`);
const refused = (error: unknown) =>
    error instanceof GnoblinError ? `code=${error.code} target=${error.target}` : `not a GnoblinError: ${error}`;

const gnoblin = Gnoblin.get();
const input = gnoblin.input;
if (input === undefined) {
    say("the compositor has no input");
} else {
    // The first context takes the name; a second with the same name is refused.
    input.addContext(new InputContext("errors"));
    try {
        input.addContext(new InputContext("errors"));
        say("a taken name was accepted");
    } catch (error) {
        say(`a taken name is refused: ${refused(error)}`);
    }

    // A context with no name is a bad argument: a TypeError, which is not a GnoblinError and has no code.
    try {
        input.addContext(new InputContext(""));
        say("an empty name was accepted");
    } catch (error) {
        say(`a bad argument is refused: TypeError=${error instanceof TypeError && !(error instanceof GnoblinError)}`);
    }
}

// A window that has closed is still a `Client`; a call on it says it is gone.
gnoblin.on("clientClosed", (client) => {
    try {
        client.close();
        say(`${client.title}: a closed window was answered`);
    } catch (error) {
        say(`${client.title}: a closed window is refused: ${refused(error)}`);
    }
});

say("running");
