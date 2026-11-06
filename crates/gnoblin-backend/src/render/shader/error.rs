//! The error a shader reports: which source, which stage, which line.

use std::fmt;

/// Where a shader's source came from, for a report that names it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ShaderOrigin {
    /// A pass the compositor ships.
    Effect(&'static str),
    /// A shader a caller registered.
    User,
}

impl fmt::Display for ShaderOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Effect(name) => write!(formatter, "the {name} pass"),
            Self::User => write!(formatter, "the user shader"),
        }
    }
}

/// The stage of a shader that failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stage {
    /// Parsing or validating the WGSL module.
    Module,
    /// Compiling the vertex shader.
    Vertex,
    /// Compiling the fragment shader.
    Fragment,
    /// Linking the two stages.
    Link,
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Module => write!(formatter, "the shader module"),
            Self::Vertex => write!(formatter, "the vertex stage"),
            Self::Fragment => write!(formatter, "the fragment stage"),
            Self::Link => write!(formatter, "the program link"),
        }
    }
}

/// A shader the compiler or the driver refused, at the line of the source that was written.
///
/// It is a value rather than a logged string because a caller has to be able to *report* it
/// — once, with the line — and because a failure must not be indistinguishable from a pass
/// that chose to draw nothing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ShaderError {
    context: ShaderOrigin,
    stage: Stage,
    line: usize,
    column: usize,
    detail: String,
}

impl ShaderError {
    pub(crate) fn new(context: ShaderOrigin, stage: Stage, line: usize, column: usize, detail: String) -> Self {
        Self {
            context,
            stage,
            line,
            column,
            detail,
        }
    }

    /// The 1-based line of the source that was written, or 0 when the failure had no
    /// position — a uniform type mismatch fails on no line at all.
    #[must_use]
    pub fn line(&self) -> usize {
        self.line
    }

    /// The 1-based column, or 0 when there is none.
    #[must_use]
    pub fn column(&self) -> usize {
        self.column
    }

    /// The compiler's or the driver's message.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// A refusal that has no line to point at: a value or a schema that disagrees with the
    /// shader the caller registered.
    pub(crate) fn refusal(detail: String) -> Self {
        Self::new(ShaderOrigin::User, Stage::Module, 0, 0, detail)
    }
}

impl fmt::Display for ShaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(formatter, "{}: {}", self.context, self.detail)
        } else if self.column == 0 {
            write!(formatter, "{} line {}: {}", self.context, self.line, self.detail)
        } else {
            write!(
                formatter,
                "{} line {}, column {}: {}",
                self.context, self.line, self.column, self.detail
            )
        }
    }
}

impl std::error::Error for ShaderError {}
