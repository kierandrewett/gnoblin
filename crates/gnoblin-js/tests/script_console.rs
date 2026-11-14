//! The `console` global: `console.log` and its siblings exist in every generation, format like Node, log through the
//! `script` area at the level they name, say where they were called from in `.ts` coordinates, and cannot flood the log.
//!
//! The first half runs a bare [`JsGeneration`] (no shell, no link), the second the shell over the fake compositor. What
//! a script logged is read from the log itself, with the level each line was logged at (`logged_with_levels`), so what
//! is proved is what `gnoblinctl logs` would show.

mod common;

use std::fs;
use std::path::Path;
use std::sync::{Arc, Once};
use std::time::Duration;

use gnoblin_js::{JsConfig, JsGeneration, JsManualClock, MAX_LINE_BYTES, MAX_LINES_PER_SECOND};
use gnoblin_log::Level;
use gnoblin_log::event::capture::logged_with_levels;
use gnoblin_log::filter::{Spec, set_spec};

use common::rig_with;

/// One line a script logged through `console`: its level and its message (the site, `: `, the text).
type Line = (Level, String);

/// What `run` logged in the `script` area of the `js` component. The log's default level is info, so the level of the
/// component is turned down to debug first, once for the process: `console.debug` is a record only when it is on.
fn script_lines(run: impl FnOnce()) -> Vec<Line> {
    static DEBUG_ON: Once = Once::new();
    DEBUG_ON.call_once(|| set_spec(Spec::parse("js:debug").expect("a valid level specification")));
    logged_with_levels(run)
        .into_iter()
        .filter(|(_, event)| {
            event.component == "js"
                && event
                    .fields
                    .iter()
                    .any(|(name, value)| name == "area" && value == "\"script\"")
        })
        .map(|(level, event)| (level, event.message))
        .collect()
}

/// The text of a line, without the call site.
fn text_of(line: &Line) -> &str {
    line.1.split_once(": ").map_or(line.1.as_str(), |(_, text)| text)
}

/// The call site of a line, `file:line:col`.
fn site_of(line: &Line) -> &str {
    line.1.split_once(": ").map_or("", |(site, _)| site)
}

/// Runs `init.ts` with `source` in a bare generation whose clock is `clock`, and returns what it logged.
fn run_init(source: &str, clock: &JsManualClock) -> Vec<Line> {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), source).expect("init.ts is written");
    let mut config = JsConfig::new(&root);
    config.clock = Arc::new(clock.clone());
    script_lines(|| {
        let generation = JsGeneration::new(&config, 1).expect("a generation");
        generation.eval_module(Path::new("init.ts")).expect("init.ts runs");
    })
}

fn run(source: &str) -> Vec<Line> {
    run_init(source, &JsManualClock::default())
}

#[test]
fn a_bare_generation_has_a_console_and_each_method_logs_at_its_level() {
    let lines =
        run("console.log('a');\nconsole.info('b');\nconsole.warn('c');\nconsole.error('d');\nconsole.debug('e');\n");
    let seen: Vec<(Level, &str)> = lines.iter().map(|line| (line.0, text_of(line))).collect();
    assert_eq!(
        seen,
        [
            (Level::INFO, "a"),
            (Level::INFO, "b"),
            (Level::WARN, "c"),
            (Level::ERROR, "d"),
            (Level::DEBUG, "e"),
        ]
    );
}

#[test]
fn the_call_site_is_the_ts_file_line_and_column_the_author_wrote() {
    // The type annotation is stripped, so the argument `n` is at column 32 of the JavaScript and 40 of the `.ts`. The
    // engine reports a call at its end and the source map takes that to the nearest token before it: `n` in the first
    // call, and `warn` in the second, where the argument is a string, which the map has no token for.
    let lines = run("const n: number = 1; console.log('hi', n);\n\n  console.warn('there');\n");
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(text_of(&lines[0]), "hi 1");
    assert_eq!(
        site_of(&lines[0]),
        "init.ts:1:40",
        "the file is shown relative to the config root"
    );
    assert_eq!(site_of(&lines[1]), "init.ts:3:11");
}

#[test]
fn the_example_of_the_debugging_page_logs_where_the_page_says() {
    // `docs/typescript-shell.md`, "Debugging": the call is on column 9, its last argument starts at column 53.
    let lines = run(
        "class Trace {\n    onFocusChanged(client: { appId: string }, focused: boolean) {\n        \
         console.log(\"%s focus is %j\", client.appId, focused);\n    }\n}\n\
         new Trace().onFocusChanged({ appId: 'foot' }, true);\n",
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(site_of(&lines[0]), "init.ts:3:53");
    assert_eq!(text_of(&lines[0]), "foot focus is true");
}

#[test]
fn strings_numbers_and_the_format_specifiers_print_like_node() {
    let lines = run("console.log('plain', 1, -0, true, null, undefined);\n\
         console.log('%s is %d and %i and %f, %j, %o, %%, %c.', 'Bob', 42.5, '7.9x', '1.5', { a: [1] }, 'x', 'css');\n\
         console.log('%s');\n\
         console.log('%d', 1, 'rest', { k: 'v' });\n\
         console.log(7, 'seven');\n");
    let texts: Vec<&str> = lines.iter().map(text_of).collect();
    assert_eq!(
        texts,
        [
            "plain 1 -0 true null undefined",
            "Bob is 42.5 and 7 and 1.5, {\"a\":[1]}, \"x\", %, .",
            "%s",
            "1 rest {\"k\":\"v\"}",
            "7 seven",
        ]
    );
}

#[test]
fn objects_arrays_and_cycles_print_as_json_with_a_circular_marker() {
    let lines = run(
        "const loop: any = { name: 'loop', list: [1, 'two', undefined, () => 1] };\n\
         loop.self = loop;\n\
         console.log(loop);\n\
         console.log([new Map([['k', 1]]), new Set([2]), new Date(0), /x/g, NaN]);\n\
         class Point { x = 1; }\n\
         console.log(new Point(), { deep: { a: { b: { c: { d: { e: { f: 1 } } } } } } });\n",
    );
    let texts: Vec<&str> = lines.iter().map(text_of).collect();
    assert_eq!(
        texts[0],
        "{\"name\":\"loop\",\"list\":[1,\"two\",undefined,[Function (anonymous)]],\"self\":[Circular]}"
    );
    assert_eq!(
        texts[1],
        "[Map(1) {\"k\" => 1},Set(1) {2},\"1970-01-01T00:00:00.000Z\",/x/g,NaN]"
    );
    assert_eq!(
        texts[2],
        "Point {\"x\":1} {\"deep\":{\"a\":{\"b\":{\"c\":{\"d\":{\"e\":[Object]}}}}}}"
    );
}

#[test]
fn an_error_prints_its_message_and_a_stack_in_ts_coordinates() {
    let lines = run(
        "function fail(): never {\n    throw new Error('boom');\n}\ntry { fail(); } catch (error) { console.error('caught', error); }\n",
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].0, Level::ERROR);
    let text = text_of(&lines[0]);
    assert!(text.starts_with("caught Error: boom\n"), "{text}");
    // Frame lines name the `.ts` coordinates: the throw is on line 2 and the call on line 4.
    assert!(text.contains("at fail (") && text.contains("init.ts:2:"), "{text}");
    assert!(text.contains("init.ts:4:"), "{text}");
}

#[test]
fn assert_logs_only_when_the_condition_fails_and_table_dumps_the_value() {
    let lines = run("console.assert(true, 'not shown');\n\
         console.assert(1 === 2, 'one is not %s', 'two');\n\
         console.assert(false);\n\
         console.table([{ id: 1, name: 'a' }, { id: 2, name: 'b' }]);\n");
    let seen: Vec<(Level, &str)> = lines.iter().map(|line| (line.0, text_of(line))).collect();
    assert_eq!(
        seen,
        [
            (Level::ERROR, "Assertion failed: one is not two"),
            (Level::ERROR, "Assertion failed"),
            (Level::INFO, "[{\"id\":1,\"name\":\"a\"},{\"id\":2,\"name\":\"b\"}]"),
        ]
    );
}

#[test]
fn a_console_call_never_throws_even_for_a_value_that_cannot_be_printed() {
    let lines = run(
        "const hostile = { get a() { throw new Error('getter'); }, toString() { throw new Error('text'); } };\n\
         console.log(hostile);\n\
         console.log('%s', hostile);\n\
         const { log } = console;\n\
         log('detached');\n\
         console.log(Symbol('s'), function named() {});\n",
    );
    let texts: Vec<&str> = lines.iter().map(text_of).collect();
    assert_eq!(texts.len(), 4, "{texts:?}");
    assert_eq!(
        texts[0],
        "{\"a\":[Getter threw: getter],\"toString\":[Function: toString]}"
    );
    assert!(texts[1].starts_with('{'), "{}", texts[1]);
    assert_eq!(texts[2], "detached");
    assert_eq!(texts[3], "Symbol(s) [Function: named]");
}

#[test]
fn one_call_logs_at_most_the_cap_and_says_how_much_it_cut() {
    let lines = run("console.log('x'.repeat(10000));\nconsole.log('é'.repeat(5000));\n");
    assert_eq!(lines.len(), 2);
    let first = text_of(&lines[0]);
    assert_eq!(
        first,
        format!(
            "{}... ({} more bytes)",
            "x".repeat(MAX_LINE_BYTES),
            10000 - MAX_LINE_BYTES
        )
    );
    let second = text_of(&lines[1]);
    assert!(second.starts_with('é') && second.ends_with("bytes)"), "{second}");
    assert!(
        second.len() < MAX_LINE_BYTES + 32,
        "a multi-byte text is cut on a character boundary"
    );
}

#[test]
fn a_loop_cannot_flood_the_log_and_the_next_second_reports_what_it_dropped() {
    let clock = JsManualClock::default();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("init.ts"),
        "for (let i = 0; i < 1000; i += 1) console.log('line', i);\n(globalThis as any).again = () => console.log('again');\n",
    )
    .expect("init.ts");
    let mut config = JsConfig::new(&root);
    config.clock = Arc::new(clock.clone());
    let generation = JsGeneration::new(&config, 1).expect("a generation");

    let first = script_lines(|| generation.eval_module(Path::new("init.ts")).expect("init.ts runs"));
    assert_eq!(first.len(), MAX_LINES_PER_SECOND, "the rest of the loop was dropped");
    assert_eq!(
        text_of(&first[MAX_LINES_PER_SECOND - 1]),
        format!("line {}", MAX_LINES_PER_SECOND - 1)
    );

    // Still the same second: nothing more is logged, and the count is not reported yet.
    clock.advance(Duration::from_millis(900));
    let same = script_lines(|| {
        generation
            .call_budgeted(Duration::from_secs(5), |ctx| ctx.eval::<(), _>("again()"))
            .expect("again runs");
    });
    assert!(same.is_empty(), "{same:?}");

    // The next second: one notice with the count, then the call goes through.
    clock.advance(Duration::from_millis(200));
    let next = script_lines(|| {
        generation
            .call_budgeted(Duration::from_secs(5), |ctx| ctx.eval::<(), _>("again()"))
            .expect("again runs");
    });
    assert_eq!(next.len(), 2, "{next:?}");
    assert_eq!(next[0].0, Level::WARN);
    assert_eq!(
        next[0].1,
        format!(
            "{} console lines dropped: a generation logs at most {MAX_LINES_PER_SECOND} a second",
            801
        )
    );
    assert_eq!(text_of(&next[1]), "again");
}

// ---- the shell ------------------------------------------------------------------------------------------------------

#[test]
fn a_shell_logs_what_init_ts_prints_and_the_notice_arrives_without_another_call() {
    let mut rig = None;
    let lines = script_lines(|| {
        rig = Some(rig_with(
            "import { gnoblin } from 'gnoblin';\n",
            "console.log('booting', gnoblin.clients.length);\n\
             for (let i = 0; i < 300; i += 1) console.log('spam', i);\n\
             (globalThis as any).fromListener = () => console.log('late');\n",
        ));
    });
    let mut rig = rig.expect("the rig booted");
    assert_eq!(
        lines.len(),
        MAX_LINES_PER_SECOND,
        "the boot printed 200 of its 301 lines"
    );
    assert_eq!(text_of(&lines[0]), "booting 0");
    assert_eq!(site_of(&lines[0]), "init.ts:2:24");

    // The clock the scripts read is the one the limit counts on: moving it past the second is enough, and the frame
    // that follows reports the 101 dropped lines without the script calling `console` again.
    rig.advance(1100);
    let report = script_lines(|| {
        rig.frame();
    });
    assert_eq!(report.len(), 1, "{report:?}");
    assert_eq!(report[0].0, Level::WARN);
    assert!(report[0].1.starts_with("101 console lines dropped"), "{}", report[0].1);

    // A console expression evaluated by the host runs in the same generation, with the same global.
    let later = script_lines(|| {
        rig.eval("fromListener(); 'done'");
    });
    assert_eq!(later.len(), 1, "{later:?}");
    assert_eq!(text_of(&later[0]), "late");
    assert_eq!(
        site_of(&later[0]),
        "init.ts:4:50",
        "the token of `log` in `console.log`"
    );
}
