//! What the typed classes `gnoblin-sdkgen` writes over [`client`](crate::client) share.
//!
//! A generated class holds an [`Object`](crate::client::Object) and calls through it; this module is the small part that does not depend on
//! a catalog: the conversion of a Rust value to and from [`Val`], the lazily resolved ids of a class, and a listener that ends when it is
//! dropped. Nothing here names a class.

use std::cell::OnceCell;

use gnoblin_core_abi as abi;

use crate::client::{Api, Reg, closure};
use crate::error::{Fail, Res};
use crate::value::{ClassId, MemberId, Val};

/// A value of a type the catalog names, as the value to send.
pub trait ToVal {
    /// The value that crosses.
    fn to_val(&self) -> Val;
}

/// A value of a type the catalog names, read from what the library sent. `C` is the generated handle on a namespace
/// (`Objects`), which a handle or a record holding one needs to adopt it.
pub trait FromVal<C>: Sized {
    /// Reads `value`. A reference an object value carries is taken over by the result.
    ///
    /// # Errors
    ///
    /// The value is not of the type, or an object in it is gone.
    fn from_val(cx: &C, value: &Val) -> Res<Self>;
}

impl<T: ToVal + ?Sized> ToVal for &T {
    fn to_val(&self) -> Val {
        (**self).to_val()
    }
}

impl ToVal for bool {
    fn to_val(&self) -> Val {
        Val::Bool(*self)
    }
}

impl ToVal for i64 {
    fn to_val(&self) -> Val {
        Val::Int(*self)
    }
}

impl ToVal for u64 {
    fn to_val(&self) -> Val {
        Val::Uint(*self)
    }
}

impl ToVal for f64 {
    fn to_val(&self) -> Val {
        Val::Double(*self)
    }
}

impl ToVal for str {
    fn to_val(&self) -> Val {
        Val::Text(self.to_owned())
    }
}

impl ToVal for String {
    fn to_val(&self) -> Val {
        Val::Text(self.clone())
    }
}

impl ToVal for () {
    fn to_val(&self) -> Val {
        Val::None
    }
}

impl<T: ToVal> ToVal for [T] {
    fn to_val(&self) -> Val {
        Val::List(self.iter().map(ToVal::to_val).collect())
    }
}

impl<T: ToVal> ToVal for Vec<T> {
    fn to_val(&self) -> Val {
        self.as_slice().to_val()
    }
}

impl<T: ToVal> ToVal for Option<T> {
    fn to_val(&self) -> Val {
        self.as_ref().map_or(Val::None, ToVal::to_val)
    }
}

impl<C> FromVal<C> for bool {
    fn from_val(_: &C, value: &Val) -> Res<bool> {
        value.bool()
    }
}

impl<C> FromVal<C> for i64 {
    fn from_val(_: &C, value: &Val) -> Res<i64> {
        value.int()
    }
}

impl<C> FromVal<C> for u64 {
    fn from_val(_: &C, value: &Val) -> Res<u64> {
        value.uint()
    }
}

impl<C> FromVal<C> for f64 {
    fn from_val(_: &C, value: &Val) -> Res<f64> {
        value.double()
    }
}

impl<C> FromVal<C> for String {
    fn from_val(_: &C, value: &Val) -> Res<String> {
        value.text().map(str::to_owned)
    }
}

/// A call that answers nothing: whatever the library sent is dropped, and a reference in it is released.
impl<C> FromVal<C> for () {
    fn from_val(_: &C, _: &Val) -> Res<()> {
        Ok(())
    }
}

impl<C, T: FromVal<C>> FromVal<C> for Vec<T> {
    fn from_val(cx: &C, value: &Val) -> Res<Vec<T>> {
        value.list()?.iter().map(|item| T::from_val(cx, item)).collect()
    }
}

impl<C, T: FromVal<C>> FromVal<C> for Option<T> {
    fn from_val(cx: &C, value: &Val) -> Res<Option<T>> {
        if value.is_none() {
            Ok(None)
        } else {
            T::from_val(cx, value).map(Some)
        }
    }
}

/// `any` in a schema file: the value as it crossed. The library gives only values with no object in them (none, a boolean, a number,
/// text, or a list of those), so reading one takes no reference over.
impl ToVal for Val {
    fn to_val(&self) -> Val {
        self.clone()
    }
}

impl<C> FromVal<C> for Val {
    fn from_val(_: &C, value: &Val) -> Res<Val> {
        Ok(value.clone())
    }
}

/// Opaque bytes (`bytes` in a schema file).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Octets(pub Vec<u8>);

impl ToVal for Octets {
    fn to_val(&self) -> Val {
        Val::Bytes(self.0.clone())
    }
}

impl<C> FromVal<C> for Octets {
    fn from_val(_: &C, value: &Val) -> Res<Octets> {
        match value {
            Val::Bytes(bytes) => Ok(Octets(bytes.clone())),
            other => Err(Fail::type_error(format!("expected bytes, got {}", other.kind_name()))),
        }
    }
}

/// The argument `index` of a call or a signal, read as a `T`. A missing one reads as none, so an optional argument is `None` and a
/// required one is an error that names the type.
///
/// # Errors
///
/// The argument is not a `T`.
pub fn arg<C, T: FromVal<C>>(cx: &C, args: &[Val], index: usize) -> Res<T> {
    T::from_val(cx, args.get(index).unwrap_or(&Val::None))
}

/// The field `index` of a record, read as a `T`; `default` makes the value when the sender left the field out.
///
/// # Errors
///
/// The field is not a `T`.
pub fn field<C, T: FromVal<C>>(cx: &C, fields: &[Val], index: usize, default: impl FnOnce() -> T) -> Res<T> {
    match fields.get(index) {
        Some(value) if !value.is_none() => T::from_val(cx, value),
        _ => Ok(default()),
    }
}

/// An object of the library that serves `namespace`, which another library handed out (`Val::Foreign`), as a value of that library's
/// own protocol: its handle, and a class that library is asked for. What the typed API of `namespace` reads it with.
///
/// # Errors
///
/// The value is not an object of another library, or that library is not the one that serves `namespace`.
pub fn foreign_in(value: &Val, namespace: &str) -> Res<Val> {
    match value {
        Val::Foreign { namespace: home, obj } if home == namespace => Ok(Val::Object { class: 0, obj: *obj }),
        Val::Foreign { namespace: home, .. } => Err(Fail::type_error(format!(
            "expected an object of `{namespace}`, got one of `{home}`"
        ))),
        other => Err(Fail::type_error(format!(
            "expected an object of `{namespace}`, got {}",
            other.kind_name()
        ))),
    }
}

/// `fail` as a failure of the call `Class.member` that has no object: a function of the class, or the making of one.
#[must_use]
pub fn at_class(fail: Fail, class: &str, member: &str) -> Fail {
    fail.at(format!("{class}.{member}"))
}

/// The id of the property `name` of `class`, which a typed class looks up once.
///
/// # Errors
///
/// The library has no such property.
pub fn property(api: &Api, class: ClassId, name: &str) -> Res<MemberId> {
    api.member(class, abi::MEMBER_PROPERTY, name)
}

/// The id of the method `name` of `class`.
///
/// # Errors
///
/// The library has no such method.
pub fn method(api: &Api, class: ClassId, name: &str) -> Res<MemberId> {
    api.member(class, abi::MEMBER_METHOD, name)
}

/// The id of the signal `name` of `class`.
///
/// # Errors
///
/// The library has no such signal.
pub fn signal(api: &Api, class: ClassId, name: &str) -> Res<MemberId> {
    api.member(class, abi::MEMBER_SIGNAL, name)
}

/// A closure value that runs `run` with the arguments of each call: what a callback property or an argument of a callback type takes.
pub fn callback(mut run: impl FnMut(&[Val]) -> Res<Val> + 'static) -> Val {
    Val::Closure(closure(move |_, args| run(args)))
}

/// A value computed once, on first use: the ids of a class, resolved by name the first time a typed call needs them.
#[derive(Debug)]
pub struct Lazy<T>(OnceCell<T>);

impl<T> Default for Lazy<T> {
    fn default() -> Self {
        Lazy(OnceCell::new())
    }
}

impl<T> Lazy<T> {
    /// The value, made by `resolve` on the first call that succeeds.
    ///
    /// # Errors
    ///
    /// `resolve` failed; the next call tries again.
    pub fn get(&self, resolve: impl FnOnce() -> Res<T>) -> Res<&T> {
        if let Some(value) = self.0.get() {
            return Ok(value);
        }
        let value = resolve()?;
        Ok(self.0.get_or_init(|| value))
    }
}

/// A listener that ends when it is dropped. [`Listener::keep`] lets it last until the generation ends instead.
#[derive(Debug)]
#[must_use = "a listener ends as soon as it is dropped; keep it, or call `keep`"]
pub struct Listener {
    api: Api,
    reg: Reg,
}

impl Listener {
    /// Holds the registration `reg` of `api`.
    pub fn new(api: Api, reg: Reg) -> Listener {
        Listener { api, reg }
    }

    /// Leaves the listener in place until the generation ends.
    pub fn keep(self) {
        std::mem::forget(self);
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.api.unlisten(self.reg);
    }
}
