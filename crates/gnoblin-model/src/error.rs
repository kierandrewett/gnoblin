//! Why the graph, or a request, refused something.
//!
//! These are programming errors and refusals, never a broken peer: a wire decode
//! failure belongs to the transport, and a version disagreement to the handshake.

use std::fmt;

use gnoblin_schema::Type;

use crate::types::TypeKey;
use crate::value::Value;
use gnoblin_types::{ObjectId, OperationId, OwnerId, RequestId};

/// Why the graph refused a read, a write or a call.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelError {
    /// No object has this identity, or it has been closed.
    UnknownObject { object: ObjectId },
    /// The catalog has no such object type.
    UnknownType { ty: TypeKey },
    /// The object's schema declares no such property.
    UnknownProperty { ty: TypeKey, property: String },
    /// A property the schema declares was left out of an object's initial state.
    ///
    /// Creation takes every declared property at once so an object is never
    /// observed with a missing value: a diff carries a value, and there is no way
    /// to say "unset" after the fact.
    MissingProperty { ty: TypeKey, property: String },
    /// A property name that the schema does not declare was supplied.
    UnexpectedProperty { ty: TypeKey, property: String },
    /// A value whose shape does not match the type the schema declares.
    WrongType { member: String, expected: Type, got: Value },
    /// The object's schema declares no such method.
    UnknownMethod { ty: TypeKey, method: String },
    /// A parameter the schema declares was left out of a call.
    MissingArgument { method: String, parameter: String },
    /// A parameter name the schema does not declare was supplied to a call.
    UnexpectedArgument { method: String, parameter: String },
    /// The object's schema declares no such signal.
    UnknownSignal { ty: TypeKey, signal: String },
    /// A field the schema declares was left out of a signal.
    MissingField { signal: String, field: String },
    /// A field name the schema does not declare was supplied to a signal.
    UnexpectedField { signal: String, field: String },
    /// The operation is not pending: never issued, or already completed.
    UnknownOperation { operation: OperationId },
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::UnknownObject { object } => write!(formatter, "no live object {object}"),
            ModelError::UnknownType { ty } => write!(formatter, "the schema declares no object type `{ty}`"),
            ModelError::UnknownProperty { ty, property } => {
                write!(formatter, "`{ty}` declares no property `{property}`")
            }
            ModelError::MissingProperty { ty, property } => {
                write!(formatter, "`{ty}` requires an initial value for property `{property}`")
            }
            ModelError::UnexpectedProperty { ty, property } => {
                write!(
                    formatter,
                    "`{ty}` declares no property `{property}`, so it cannot be set"
                )
            }
            ModelError::WrongType { member, expected, got } => write!(
                formatter,
                "{member} is declared `{expected}`, but the value is a {}",
                got.kind()
            ),
            ModelError::UnknownMethod { ty, method } => {
                write!(formatter, "`{ty}` declares no method `{method}`")
            }
            ModelError::MissingArgument { method, parameter } => {
                write!(formatter, "method `{method}` requires parameter `{parameter}`")
            }
            ModelError::UnexpectedArgument { method, parameter } => {
                write!(formatter, "method `{method}` declares no parameter `{parameter}`")
            }
            ModelError::UnknownSignal { ty, signal } => {
                write!(formatter, "`{ty}` declares no signal `{signal}`")
            }
            ModelError::MissingField { signal, field } => {
                write!(formatter, "signal `{signal}` requires field `{field}`")
            }
            ModelError::UnexpectedField { signal, field } => {
                write!(formatter, "signal `{signal}` declares no field `{field}`")
            }
            ModelError::UnknownOperation { operation } => {
                write!(
                    formatter,
                    "{operation} is not pending; it was never issued or has completed"
                )
            }
        }
    }
}

impl std::error::Error for ModelError {}

/// Why a request transition was refused.
///
/// Every variant is a refusal, never a silent no-op: a client that loses a claim
/// learns which owner holds the request, so it can report the slot as taken
/// instead of waiting for an answer that will never come.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// The broker has never offered this request, or has finished and forgotten it.
    Unknown { request: RequestId },
    /// Another owner holds the request.
    Taken { request: RequestId, owner: OwnerId },
    /// The request already finished. It stays finished: an answer never reopens it.
    Finished { request: RequestId },
    /// The caller is not the owner the request was claimed by.
    NotOwner { request: RequestId, owner: OwnerId },
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::Unknown { request } => write!(formatter, "{request} was never offered or is forgotten"),
            RequestError::Taken { request, owner } => write!(formatter, "{request} is already claimed by {owner}"),
            RequestError::Finished { request } => write!(formatter, "{request} has already finished"),
            RequestError::NotOwner { request, owner } => {
                write!(formatter, "{owner} does not own {request}")
            }
        }
    }
}

impl std::error::Error for RequestError {}
