// Example 38 in TypeScript: the options record of a constructor, through the classes generated from the catalogs.
//
//     gnoblin ./shell.ts
//
// A constructor takes the properties its catalog names as positional parameters, and then one optional record: each field of it sets the
// property of its name in the same call that makes the object, so a value the library refuses fails the call and leaves no object. This
// shell makes:
//
// - workspaces with no options, with every option (`name`, `gaps`, `layout`) and with one of them, and reads what each holds;
// - output profiles with no options and with the options `id` and `settings`, which the library sets as the object is made;
// - a workspace and two profiles with a value the library refuses, and reads the library's words and what the lists hold afterwards.
//
// Every line starts with `ctoropts:`, spelt as the Rust and Python shells of the same example (`38-native-options`) spell it, so that
// one test reads all three.
import { gnoblin } from "gnoblin-core/generated/objects";
import { MasterStackLayout, OutputProfile, Workspace } from "gnoblin-std/generated/objects";

function say(text: string): void {
    console.log(`ctoropts: ${text}`);
}

/** The words the library refuses `work` with, or `no` when it did not. */
function refused(work: () => unknown): string {
    try {
        work();
        return "no";
    } catch (error) {
        return (error as Error).message;
    }
}

function gapsOf(workspace: Workspace): string {
    const gaps = workspace.gaps;
    return gaps === undefined ? "none" : `${gaps.inner}/${gaps.outer}`;
}

const workspaces = gnoblin.workspaces;
const profiles = gnoblin.outputs?.profiles;
if (profiles === undefined) {
    throw new Error("the core has no outputs");
}

say("running");

// No options: the constructor of every shell so far.
const plain = new Workspace("plain");
say(`plain workspace: id=${plain.id} name="${plain.name}" gaps=${gapsOf(plain)}`);

// Every option.
const main = new Workspace("main", {
    name: "Main",
    gaps: { inner: 6, outer: 12 },
    layout: new MasterStackLayout(),
});
say(`main workspace: id=${main.id} name="${main.name}" gaps=${gapsOf(main)} layout=${main.layout !== undefined}`);

// One option: what is left out keeps the library's default.
const half = new Workspace("half", { name: "Half" });
say(`half workspace: id=${half.id} name="${half.name}" gaps=${gapsOf(half)}`);

// A value the library refuses fails the call, and the next workspace of the id is made as if nothing had happened.
say(`refused gaps: ${refused(() => new Workspace("bad", { gaps: { inner: -1, outer: 0 } }))}`);
const bad = new Workspace("bad");
say(`bad workspace after the refusal: id=${bad.id} name="${bad.name}" gaps=${gapsOf(bad)}`);

workspaces.add(main);
workspaces.add(plain);
say(`workspaces hold ${workspaces.list.map((workspace) => workspace.id).join(", ")}`);

// The options of a class whose constructor sets only the match: the library sets the rest after it makes the object.
const hdmi = { connector: "HDMI-A-9" };
const unnamed = new OutputProfile(hdmi);
say(`plain profile: id=${unnamed.id} scale=${unnamed.settings?.scale ?? "unset"}`);
const mine = new OutputProfile(hdmi, { id: "mine", settings: { scale: 2 } });
say(`profile with options: id=${mine.id} scale=${mine.settings?.scale ?? "unset"}`);
say(`refused id: ${refused(() => new OutputProfile(hdmi, { id: "" }))}`);
say(`refused scale: ${refused(() => new OutputProfile(hdmi, { id: "scaled", settings: { scale: 9 } }))}`);

profiles.add(mine);
profiles.add(unnamed);
say(`profiles hold ${profiles.length}`);
say("done");
