// Example 23 in TypeScript: the session services of libgnoblin-core, through the classes generated from its catalog.
//
//     gnoblin ./shell.ts
//
// Starts a command line, watches the session for idleness, begins a launch that no window answers and asks for the session lock with
// `Super+e`. Every line starts with `services:`, as the Rust shell of the same example spells it.
import { Gnoblin } from "gnoblin-core/generated/objects";
import { gnoblin } from "gnoblin-core";

const model = Gnoblin.get();
const version = model.version;
if (version !== undefined) {
    console.log(`services: gnoblin ${version.gnoblin} speaks api ${version.api}`);
}

const process = model.shell("printf hello; exit 3", { stdout: "capture" });
process.on("exited", (ended) => {
    console.log(
        `services: the process ended with code ${ended.exitCode} and printed ${JSON.stringify(ended.exit?.stdout ?? "")}`,
    );
});

const launch = model.launches.begin("never", "org.example.Never", { timeoutMs: 100 });
launch.on("timedOut", (late) => console.log(`services: the launch ${late.token} timed out`));

const watch = model.idle.watch(300);
watch.on("idle", () => console.log("services: the session is idle"));
watch.on("resume", () => console.log("services: the session is active again"));

gnoblin.input.primary.bind("Super+e", () => {
    const operation = model.session.lock();
    operation.on("settled", () => {
        console.log(
            `services: the lock request settled, ${operation.error === "" ? "accepted" : `refused with ${operation.errorCode}`}`,
        );
    });
});

console.log("services: running");
