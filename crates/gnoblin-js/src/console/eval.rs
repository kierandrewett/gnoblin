//! Evaluating one console request: the expression or statements are run in the generation, and the value
//! is rendered for the socket.

use std::time::Duration;

use rquickjs::context::EvalOptions;
use rquickjs::{Ctx, Function, Object, Value};
use serde_json::{Map, Value as Json, json};

use super::{CONSOLE_FILE, MAX_JOBS};
use crate::choice::StockRuntime;
use crate::error::{JsError, JsErrorKind};
use crate::generation::JsGeneration;
use crate::script_console::JsConsoleLine;
use crate::shell::JsShell;

/// How a generation on the native library renders a console value: JSON, or the text of what has none, cut to the reply's limit.
const PLAIN_RENDER: &str = "(function (value, max) { let text; try { text = typeof value === 'function' || value === undefined ? String(value) : JSON.stringify(value, (key, item) => typeof item === 'bigint' ? String(item) : item); } catch (error) { text = String(value); } if (text === undefined) { text = String(value); } return text.length > max ? text.slice(0, max) : text; })";

/// The reply to one request line.
pub(super) fn answer(shell: &JsShell, line: &[u8], max_bytes: usize) -> Json {
    let request: Json = match serde_json::from_slice(line) {
        Ok(request) => request,
        Err(error) => {
            return failure(
                &Json::Null,
                "request",
                &format!("a request is one JSON object per line: {error}"),
            );
        }
    };
    let id = request.get("id").cloned().unwrap_or(Json::Null);
    let Some(expr) = request.get("expr").and_then(Json::as_str) else {
        return failure(&id, "request", "a request needs an `expr` string");
    };
    let (generation, budget) = shell.console_target();
    let (outcome, output) = evaluate_capturing(generation, budget, expr, max_bytes);
    let mut reply = match outcome {
        Ok(rendered) => match serde_json::from_str::<Map<String, Json>>(&rendered) {
            Ok(mut reply) => {
                reply.insert("id".to_owned(), id);
                reply.insert("ok".to_owned(), Json::Bool(true));
                Json::Object(reply)
            }
            Err(error) => failure(
                &id,
                "engine",
                &format!("the renderer returned what is not JSON: {error}"),
            ),
        },
        Err(error) => error_reply(&id, &error),
    };
    // What the expression logged through `console`, in order, whether it ended well or not.
    reply["output"] = output_json(&output);
    reply
}

fn output_json(lines: &[JsConsoleLine]) -> Json {
    Json::Array(
        lines
            .iter()
            .map(|line| json!({"level": line.level.name(), "site": line.site, "text": line.text}))
            .collect(),
    )
}

pub(super) fn failure(id: &Json, kind: &str, message: &str) -> Json {
    json!({"id": id, "ok": false, "error": {"kind": kind, "message": message, "location": null, "stack": null}})
}

fn error_reply(id: &Json, error: &JsError) -> Json {
    let kind = match error.kind {
        JsErrorKind::Budget => "budget",
        JsErrorKind::Memory => "memory",
        JsErrorKind::Exception => "exception",
        JsErrorKind::Ts | JsErrorKind::Load | JsErrorKind::Engine => "engine",
    };
    // The place that matters is the one in what the person typed, not a frame of the library under it.
    let location = error
        .stack
        .as_deref()
        .and_then(console_location)
        .map(|(line, column)| json!({"file": CONSOLE_FILE, "line": line, "column": column}))
        .or_else(|| {
            error
                .location
                .as_ref()
                .map(|at| json!({"file": at.file.display().to_string(), "line": at.line, "column": at.column}))
        })
        .unwrap_or(Json::Null);
    json!({
        "id": id,
        "ok": false,
        "error": {"kind": kind, "message": error.message, "location": location, "stack": error.stack},
    })
}

/// The line and column of the first stack frame in a console expression.
pub(super) fn console_location(stack: &str) -> Option<(u32, u32)> {
    stack.lines().find_map(|frame| {
        let rest = &frame[frame.find(CONSOLE_FILE)? + CONSOLE_FILE.len()..];
        let mut parts = rest.strip_prefix(':')?.trim_end_matches(')').split(':');
        Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
    })
}

/// Evaluates `source` as a script in the running generation of `shell` and renders its value, as the text of the
/// reply's JSON. Not counted as a strike when it runs past its budget.
pub fn evaluate_in(shell: &JsShell, source: &str, max_bytes: usize) -> Result<String, JsError> {
    let (generation, budget) = shell.console_target();
    evaluate_capturing(generation, budget, source, max_bytes).0
}

/// [`evaluate`], with the lines the expression logged through `console` kept apart from the log's own record, so the
/// reply can carry them to the prompt.
fn evaluate_capturing(
    generation: &JsGeneration,
    budget: Duration,
    source: &str,
    max_bytes: usize,
) -> (Result<String, JsError>, Vec<JsConsoleLine>) {
    generation.capture_console();
    let outcome = evaluate(generation, budget, source, max_bytes);
    (outcome, generation.take_console())
}

fn evaluate(generation: &JsGeneration, budget: Duration, source: &str, max_bytes: usize) -> Result<String, JsError> {
    let source = as_expression(source);
    generation.call_budgeted(budget, |ctx| {
        // Sloppy mode, as a prompt expects: `x = 5` makes a global and does not throw.
        let mut options = EvalOptions::default();
        options.strict = false;
        options.filename = Some(CONSOLE_FILE.to_owned());
        let value: Value<'_> = ctx.eval_with_options(source, options)?;
        let settled = settle(ctx, value)?;
        if generation.runtime() == StockRuntime::Typescript {
            let host: Object<'_> = ctx.globals().get("__gnoblinHost")?;
            let entry: Object<'_> = host.get("entry")?;
            return entry
                .get::<_, Function<'_>>("render")?
                .call::<_, String>((settled, max_bytes));
        }
        // A generation on the native library has no `inspect` of its own yet: a value is shown as JSON when it has one.
        let render: Function<'_> = ctx.eval(PLAIN_RENDER)?;
        render.call::<_, String>((settled, max_bytes))
    })
}

/// A value after the promise jobs it started have run: a promise that is settled is its result (a rejected one
/// throws its reason), anything else is itself.
fn settle<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Value<'js>> {
    for _ in 0..MAX_JOBS {
        if !ctx.execute_pending_job() {
            break;
        }
    }
    match value.as_promise().map(|promise| promise.result::<Value<'js>>()) {
        Some(Some(result)) => result,
        _ => Ok(value),
    }
}

/// `source`, in parentheses when it starts with `{` and reads as an object literal that way: at a prompt `{a: 1}`
/// is the object, not a block holding a label. A `{ ... }` that is a block is left alone.
pub(super) fn as_expression(source: &str) -> String {
    if source.trim_start().starts_with('{') {
        let wrapped = format!("({source}\n)");
        if parses(&wrapped) {
            return wrapped;
        }
    }
    source.to_owned()
}

fn parses(source: &str) -> bool {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::cjs()).parse();
    parsed.diagnostics.errors().next().is_none()
}
