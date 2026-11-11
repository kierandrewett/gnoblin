//! The error a shader reports: which pass, which stage, which line.

use std::fmt;

/// The stage of a shader that failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
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
    pass: &'static str,
    stage: Stage,
    line: usize,
    column: usize,
    detail: String,
}

impl ShaderError {
    /// Builds an error for `detail` at `line` and `column` of the pass named `pass`.
    pub fn new(pass: &'static str, stage: Stage, line: usize, column: usize, detail: String) -> Self {
        Self {
            pass,
            stage,
            line,
            column,
            detail,
        }
    }

    /// The 1-based line of the source, or 0 when the failure had no position.
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
}

impl fmt::Display for ShaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(formatter, "the {} pass: {}", self.pass, self.detail)
        } else if self.column == 0 {
            write!(formatter, "the {} pass line {}: {}", self.pass, self.line, self.detail)
        } else {
            write!(
                formatter,
                "the {} pass line {}, column {}: {}",
                self.pass, self.line, self.column, self.detail
            )
        }
    }
}

impl std::error::Error for ShaderError {}
