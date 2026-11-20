//! What the classes that work on the library's own state (`style`, `transition`) share: reading the fields of a record a shell gave,
//! building one to answer, and running work over the library's state for a change or a read.

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::{Fail, Res, Val};

use crate::error::Res as StdRes;
use crate::state::State;
use crate::table::{mutate_quiet, read_quiet};

/// A field of a record: none when it is left out.
pub(crate) fn field(fields: &[Val], at: usize) -> Option<&Val> {
    fields.get(at).filter(|value| !value.is_none())
}

/// The number in field `at`, or `default` when it is left out.
pub(crate) fn number_field(fields: &[Val], at: usize, default: f64) -> Res<f64> {
    field(fields, at).map_or(Ok(default), Val::double)
}

/// The flag in field `at`, or `default` when it is left out.
pub(crate) fn flag_field(fields: &[Val], at: usize, default: bool) -> Res<bool> {
    field(fields, at).map_or(Ok(default), Val::bool)
}

/// The text in field `at`, or `default` when it is left out.
pub(crate) fn text_field(fields: &[Val], at: usize, default: &str) -> Res<String> {
    field(fields, at).map_or_else(|| Ok(default.to_owned()), |value| value.text().map(str::to_owned))
}

/// The numbers in field `at`: none when it is left out.
pub(crate) fn numbers_field(fields: &[Val], at: usize) -> Res<Vec<f64>> {
    field(fields, at).map_or_else(
        || Ok(Vec::new()),
        |value| value.list()?.iter().map(Val::double).collect(),
    )
}

/// A record with these fields, in the order of its class.
pub(crate) fn record(fields: Vec<Val>) -> Val {
    Val::Record { class: 0, fields }
}

/// A value of an enum by its place, counted from one.
pub(crate) fn enumeration(place: u32) -> Val {
    Val::Enum { class: 0, value: place }
}

/// Runs `work` over the library's state for a change, as the style and animation functions of the table do: the library's model is
/// opened alone, and a turn is posted for what the change needs done.
pub(crate) fn edit<R>(ctx: GnoblinCtx, work: impl FnOnce(&mut State) -> StdRes<R>) -> Res<R> {
    mutate_quiet(ctx, work).map_err(Fail::from)
}

/// Runs `work` over the library's state to read.
pub(crate) fn look<R>(ctx: GnoblinCtx, work: impl FnOnce(&State) -> StdRes<R>) -> Res<R> {
    read_quiet(ctx, work).map_err(Fail::from)
}
