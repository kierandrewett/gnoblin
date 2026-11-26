//! Example 37 (`examples/37-native-std-root`), run for real from the three languages: the shipped binary serving a shell that reads the
//! members the std library adds to the core's root object, and the failure modes of a member that exists only while std is loaded.
//!
//! The std catalog declares `[[augment]]` blocks for `core.gnoblin` (`workspaces`, `displays`, `scene`, `gestures`, `requestTransition`),
//! `core.output_collection` (`profiles`), and, as receiver augments that act on the object they are used on, `core.input_context`
//! (`activeDisplay`) and `core.client_collection` (`style`). The core catalog never reads them, so:
//!
//! - a shell of each language that loads no std library has none of the members, and the core-only Rust shell of example 35 records no
//!   `NEEDED libgnoblin-std` (`readelf -d`), which is the proof the core carries no symbol of std;
//! - a shell that loads std reads `gnoblin.workspaces`, shows a workspace, replaces a display, reads the output profiles and runs
//!   `requestTransition`, and what the checks refuse is refused with the library's words;
//! - after a shell reload the members belong to the new generation's std library: the list the new generation reads is its own and holds
//!   nothing the old generation added.

use std::fs;
use std::path::Path;

use crate::support::elf::assert_shell_dependencies;
use crate::support::native_shell::{Gnoblin, Start, shells, start_script, std_shells};
use crate::{copy_example, save, serving};

use super::native_std::{assert_one_core_one_std, start_std};

/// What a language the example is written in is called in a failure.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    TypeScript,
    Python,
}

/// The session of the shell of the example in `language`.
fn session(language: Language) -> (Option<tempfile::TempDir>, Gnoblin) {
    match language {
        Language::Rust => (None, start_std(&std_shells().std_root)),
        Language::TypeScript | Language::Python => {
            let dir = copy_example("37-native-std-root");
            let file = if language == Language::TypeScript {
                "shell.ts"
            } else {
                "shell.py"
            };
            let mut session = start_script(Start::Compositor, &dir.path().join(file), &[]);
            serving(&mut session);
            (Some(dir), session)
        }
    }
}

/// The line the shell logs that starts with `stdroot: ` and then `text`.
#[track_caller]
fn said(session: &mut Gnoblin, text: &str) -> String {
    let wanted = format!("stdroot: {text}");
    session
        .wait_log(&format!("the shell said `{wanted}`"), |line| line.contains(&wanted))
        .text
}

#[track_caller]
fn the_root_has_the_std_members(language: Language, session: &mut Gnoblin) {
    said(session, "running");
    // The list is the one the root's `workspaces` member reads, and holds what the shell added.
    said(session, "workspaces hold main, side");
    said(session, "scene metres per unit is 1: true");
    said(session, "gestures swipe threshold is positive: true");
    said(session, "requestTransition ran its work: true");
    // A transition that cannot start is refused with the library's words about the duration.
    let refused = said(session, "requestTransition refused:");
    assert!(refused.contains("duration"), "{refused}");
    // The style of every window, and a scope of its own for a selection and a selection of it; a list made by the shell has none.
    said(session, "clients style opacity is 0.9: true");
    said(session, "a selection has its own style: true");
    said(session, "a selection of a selection has its own style: true");
    let list = said(session, "refused list style:");
    assert!(list.contains("has no style"), "a list made by the shell: {list}");

    // With a display for the output, the area is known and a workspace is shown by itself and by its id.
    said(session, "the area is known: true");
    said(session, "show side: true");
    said(session, "side is active: true");
    said(session, "show main by id: true");
    said(session, "main is active: true");
    // The display under the pointer, else the primary one, shows the workspace that was shown last.
    said(session, "the active display shows main: true");
    let ghost = said(session, "refused ghost:");
    assert!(
        ghost.contains("add the workspace to the list first"),
        "a workspace that is not in the list: {ghost}"
    );
    let nope = said(session, "refused nope:");
    assert!(
        nope.contains("no workspace \"nope\" in the list"),
        "an id that is not in the list: {nope}"
    );

    // The profiles of the outputs and the displays: a display of the shell's own is replaced by another, a display of the library's is not.
    said(session, "displays hold 1");
    said(session, "profiles hold 1");
    let library = said(session, "refused library display:");
    assert!(
        library.contains("is not a display of the script"),
        "the library's display stays: {library}"
    );
    said(session, "displays hold mine");
    said(session, "replaced mine with next: next");
    said(session, "removed next: true");
    said(session, "after removing, displays hold 1");
    said(session, "removed profile: true");
    said(session, "profiles hold 0");
    said(session, "done");

    if language == Language::Rust {
        assert_one_core_one_std(session);
    } else {
        assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
    }
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_37_a_std_shell_reads_the_members_std_adds_to_the_core_root() {
    let (_dir, mut session) = session(Language::Rust);
    the_root_has_the_std_members(Language::Rust, &mut session);
}

#[test]
fn example_37_a_typescript_shell_reads_the_members_std_adds_to_the_core_root() {
    let (_dir, mut session) = session(Language::TypeScript);
    the_root_has_the_std_members(Language::TypeScript, &mut session);
}

#[test]
fn example_37_a_python_shell_reads_the_members_std_adds_to_the_core_root() {
    let (_dir, mut session) = session(Language::Python);
    the_root_has_the_std_members(Language::Python, &mut session);
}

// ---- a shell that loads no std has none of the members --------------------------------------------------------------------------

/// Failure mode: the core-only Rust shell of example 35 records `NEEDED libgnoblin-core.so.1` and no std library, so `readelf -d` shows
/// none, and the std shell of example 37 records both.
#[test]
fn a_core_only_rust_shell_records_no_std_library_and_a_std_shell_records_it() {
    assert_shell_dependencies(&shells().root, false);
    assert_shell_dependencies(&std_shells().std_root, true);
}

/// Which of the members a shell has, as the script of a language reads them.
const MEMBERS: [&str; 5] = ["workspaces", "displays", "scene", "gestures", "requestTransition"];

/// A TypeScript shell that says which members `gnoblin` and its outputs have, after importing `imports`.
fn typescript_shell(imports: &str) -> String {
    format!(
        "import {{ gnoblin }} from \"gnoblin-core/generated/objects\";\n{imports}\n\
         const names = {names:?};\n\
         const have = names.map((name) => `${{name}}=${{name in gnoblin}}`).join(\" \");\n\
         const outputs = gnoblin.outputs;\n\
         console.log(`members: ${{have}} profiles=${{outputs !== undefined && \"profiles\" in outputs}}`);\n",
        names = MEMBERS
    )
}

/// A Python shell that says which members `Gnoblin` and its output collections have, after importing `imports`.
fn python_shell(imports: &str) -> String {
    format!(
        "from gnoblin import gnoblin as session\n\
         from gnoblin.objects.model import Gnoblin, OutputCollection\n{imports}\n\
         names = [{names}]\n\
         have = \" \".join(f\"{{name}}={{str(hasattr(Gnoblin, name)).lower()}}\" for name in names)\n\
         session.log(f\"members: {{have}} profiles={{str(hasattr(OutputCollection, 'profiles')).lower()}}\")\n",
        names = MEMBERS.map(|name| format!("{:?}", python_name(name))).join(", ")
    )
}

/// The Python spelling of a member.
fn python_name(name: &str) -> String {
    match name {
        "requestTransition" => "request_transition".to_owned(),
        other => other.to_owned(),
    }
}

/// The line a members shell logs, once it has.
#[track_caller]
fn members(language: Language, imports: &str) -> String {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let (file, text) = match language {
        Language::TypeScript => ("shell.ts", typescript_shell(imports)),
        _ => ("shell.py", python_shell(imports)),
    };
    let shell: &Path = &dir.path().join(file);
    fs::write(shell, text).expect("the shell");
    let mut session = start_script(Start::Compositor, shell, &[]);
    serving(&mut session);
    let line = session
        .wait_log("the members were read", |line| line.contains("members: "))
        .text;
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
    line
}

/// Failure mode: with no import of std, `gnoblin` has no `workspaces` (the attribute is missing), and nothing else std adds either.
#[test]
fn a_typescript_shell_that_loads_no_std_has_none_of_the_members() {
    let line = members(Language::TypeScript, "");
    for name in MEMBERS {
        assert!(line.contains(&format!("{name}=false")), "{name}: {line}");
    }
    assert!(line.contains("profiles=false"), "{line}");
}

#[test]
fn a_python_shell_that_loads_no_std_has_none_of_the_members() {
    let line = members(Language::Python, "");
    for name in MEMBERS {
        assert!(line.contains(&format!("{}=false", python_name(name))), "{name}: {line}");
    }
    assert!(line.contains("profiles=false"), "{line}");
}

/// A shell whose only stock import is `gnoblin-std/generated/objects` runs on the generated library and not on the TypeScript one: the
/// TypeScript library installs the global `gnoblin`, the generated classes do not, and the std class it names works.
#[test]
fn a_typescript_shell_whose_only_stock_import_is_the_std_module_runs_on_the_generated_library() {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let shell = dir.path().join("shell.ts");
    fs::write(
        &shell,
        "import { Workspaces } from \"gnoblin-std/generated/objects\";\n\
         console.log(`stdonly: global gnoblin is ${typeof (globalThis as any).gnoblin}, workspaces ${Workspaces.get().length}`);\n",
    )
    .expect("the shell");
    let mut session = start_script(Start::Compositor, &shell, &[]);
    serving(&mut session);
    let line = session
        .wait_log("the shell read the list", |line| line.contains("stdonly: "))
        .text;
    assert!(
        line.contains("global gnoblin is undefined"),
        "it ran on the TypeScript library: {line}"
    );
    assert!(!line.contains("workspaces NaN"), "{line}");
    assert!(session.exited().is_none(), "{}", session.log_text());
}

/// The same shell that imports std's module has every member: importing it is what puts them on the core's class.
#[test]
fn importing_the_std_module_puts_the_members_on_the_core_class_in_typescript() {
    let line = members(Language::TypeScript, "import \"gnoblin-std/generated/objects\";");
    for name in MEMBERS {
        assert!(line.contains(&format!("{name}=true")), "{name}: {line}");
    }
    assert!(line.contains("profiles=true"), "{line}");
}

#[test]
fn importing_the_std_module_puts_the_members_on_the_core_class_in_python() {
    let line = members(Language::Python, "import gnoblin.objects.std");
    for name in MEMBERS {
        assert!(line.contains(&format!("{}=true", python_name(name))), "{name}: {line}");
    }
    assert!(line.contains("profiles=true"), "{line}");
}

// ---- after a reload the members belong to the new generation ---------------------------------------------------------------------

/// A shell of the generation `tag`: it adds a workspace named `tag` and says which workspaces the root's list holds.
fn reload_shell(language: Language, tag: &str) -> String {
    match language {
        Language::TypeScript => format!(
            "import {{ gnoblin }} from \"gnoblin-core/generated/objects\";\n\
             import {{ Workspace }} from \"gnoblin-std/generated/objects\";\n\
             gnoblin.workspaces.add(new Workspace(\"{tag}\"));\n\
             console.log(`stdreload: {tag} holds ${{gnoblin.workspaces.list.map((workspace) => workspace.id).join(\",\")}}`);\n"
        ),
        _ => format!(
            "from gnoblin import gnoblin as session\n\
             from gnoblin.objects.model import Gnoblin\n\
             from gnoblin.objects.std import Workspace\n\n\
             gnoblin = Gnoblin.get()\n\
             gnoblin.workspaces.add(Workspace(\"{tag}\"))\n\
             session.log(\"stdreload: {tag} holds \" + \",\".join(workspace.id for workspace in gnoblin.workspaces.list))\n"
        ),
    }
}

/// Failure mode: a member read after a reload is the new generation's. Each generation adds a workspace of its own and reads the list
/// through the root: the first holds its own, the second holds its own and not the first's, which the retired std library held.
#[track_caller]
fn the_members_rebind_after_a_reload(language: Language) {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let file = if language == Language::TypeScript {
        "shell.ts"
    } else {
        "shell.py"
    };
    let shell = dir.path().join(file);
    fs::write(&shell, reload_shell(language, "first")).expect("the shell");
    let mut session = start_script(Start::Compositor, &shell, &[]);
    serving(&mut session);
    session.wait_log("the first generation read its own list", |line| {
        line.contains("stdreload: first holds first")
    });

    save(&shell, &reload_shell(language, "second"));
    let line = session
        .wait_log("the second generation read the root's list", |line| {
            line.contains("stdreload: second holds")
        })
        .text;
    assert!(
        line.contains("stdreload: second holds second"),
        "the list of the new generation holds its own workspace and not the old one's: {line}"
    );

    // A third generation: still its own library, after two reloads.
    save(&shell, &reload_shell(language, "third"));
    let line = session
        .wait_log("the third generation read the root's list", |line| {
            line.contains("stdreload: third holds")
        })
        .text;
    assert!(line.contains("stdreload: third holds third"), "{line}");
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn a_typescript_shells_std_members_belong_to_the_new_generation_after_a_reload() {
    the_members_rebind_after_a_reload(Language::TypeScript);
}

#[test]
fn a_python_shells_std_members_belong_to_the_new_generation_after_a_reload() {
    the_members_rebind_after_a_reload(Language::Python);
}
