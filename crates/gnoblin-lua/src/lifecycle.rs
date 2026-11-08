//! The events the host raises when a core object opens, changes or closes.
//!
//! A `window` and an `output` each have lifecycle signals that the schema declares
//! (`window.created`, `window.changed`, `output.added`, `output.changed`,
//! `output.removed`). The compositor does not send them: the host derives them from the
//! graph's observations, and the payload is read from the object's mirrored view. Each
//! function returns the signal to raise and its fields, or `None` when the object is not
//! one these events describe.

use gnoblin_model::{Fields, ObjectId, ObjectView, Value as ModelValue};

const NAMESPACE: &str = "gnoblin";

/// The signal to raise and its payload.
pub(crate) type Raised = (&'static str, Fields);

/// The event an object raises when the graph opens it, if any.
pub(crate) fn opened(object: ObjectId, view: &ObjectView) -> Option<Raised> {
    if !is_core(view, "window") && !is_core(view, "output") && !is_core(view, "workspace") {
        return None;
    }
    if is_core(view, "window") {
        return Some(("created", window_fields(object)));
    }
    if is_core(view, "workspace") {
        return Some((
            "created",
            Fields::from([("workspace".to_owned(), ModelValue::Object(object))]),
        ));
    }
    let output_id = output_id(view)?;
    Some((
        "added",
        Fields::from([
            ("output_id".to_owned(), output_id),
            ("output".to_owned(), ModelValue::Object(object)),
        ]),
    ))
}

/// The event an object raises when one of its properties changes, if any. A window's
/// `focused` flip raises its own events instead; the host checks those first.
pub(crate) fn changed(object: ObjectId, view: &ObjectView, property: String) -> Option<Raised> {
    if is_core(view, "window") {
        let window_id = match view.properties().get("id") {
            Some(ModelValue::Uint(id)) => ModelValue::Uint(*id),
            _ => return None,
        };
        return Some((
            "changed",
            Fields::from([
                ("window_id".to_owned(), window_id),
                ("changed".to_owned(), ModelValue::List(vec![ModelValue::Text(property)])),
                ("window".to_owned(), ModelValue::Object(object)),
            ]),
        ));
    }
    if is_core(view, "output") {
        let output_id = output_id(view)?;
        return Some((
            "changed",
            Fields::from([
                ("output_id".to_owned(), output_id),
                ("output".to_owned(), ModelValue::Object(object)),
                ("changed".to_owned(), ModelValue::List(vec![ModelValue::Text(property)])),
            ]),
        ));
    }
    None
}

/// The event an object raises when the graph closes it, if any. The handlers still
/// read the object: the mirror entry is dropped after they run.
pub(crate) fn closed(object: ObjectId, view: &ObjectView) -> Option<Raised> {
    if is_core(view, "workspace") {
        let workspace_id = match view.properties().get("id") {
            Some(ModelValue::Uint(id)) => ModelValue::Uint(*id),
            _ => return None,
        };
        return Some((
            "removed",
            Fields::from([
                ("workspace_id".to_owned(), workspace_id),
                ("last".to_owned(), ModelValue::Object(object)),
            ]),
        ));
    }
    if !is_core(view, "output") {
        return None;
    }
    let output_id = output_id(view)?;
    Some((
        "removed",
        Fields::from([
            ("output_id".to_owned(), output_id),
            ("last".to_owned(), ModelValue::Object(object)),
        ]),
    ))
}

fn is_core(view: &ObjectView, name: &str) -> bool {
    view.ty().namespace() == NAMESPACE && view.ty().name() == name
}

/// An output's identity is its connector name, the value `by_id` takes.
fn output_id(view: &ObjectView) -> Option<ModelValue> {
    match view.properties().get("name") {
        Some(ModelValue::Text(name)) => Some(ModelValue::Text(name.clone())),
        _ => None,
    }
}

fn window_fields(object: ObjectId) -> Fields {
    Fields::from([("window".to_owned(), ModelValue::Object(object))])
}
