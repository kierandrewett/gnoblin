//! The validator of the user shader pass: a shell's WGSL source in, a refusal with a line and a column, or the program the
//! renderers compile out.
//!
//! This is renderer independent on purpose. The compositor runs it when a shell writes an `effects` list, so a bad shader is
//! refused at write time on every renderer; a renderer runs it again when it compiles ([`Validated::wgsl`]), and that second
//! run is what stops a pass that reached the frame some other way.
//!
//! # The contract
//!
//! The shell writes one function and may write constants, helper functions and structs:
//!
//! ```wgsl
//! fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32>
//! ```
//!
//! - `c` is the node's own premultiplied colour at this point of the layer;
//! - `uv` is this point as a coordinate over the layer's rectangle, `(0, 0)` at the top-left;
//! - `p` is the pass's numbers (at most eight, the ones not written are zero);
//! - `t` is the pass's time in seconds;
//! - the result is a premultiplied colour, which the wrapper clamps, tints, fades by the layer's opacity and cuts to the
//!   layer's rounded silhouette.
//!
//! The wrapper adds `fn backdrop(uv: vec2<f32>) -> vec4<f32>`, which the shell may call: the premultiplied colour of what is
//! behind the layer at `uv`, with the same coordinates as `effect`'s `uv`. A coordinate outside `0..1` reads the nearest edge
//! pixel. What `backdrop` reads is [`OutputFrame::backdrop_input`](crate::frame::OutputFrame::backdrop_input): the layers below
//! the pass's layer with every secure window and the lock card drawn black. There is no other way to read another window, and
//! the pass has no way to write anywhere but its own pixel, so nothing it computes reaches the shell.
//!
//! The wrapper owns every binding. The source may not declare a variable, an override, a texture, a sampler, a buffer or an
//! entry point, may not use pointers as parameters, and may not use a name the wrapper owns (`shade`, `backdrop`,
//! `GnoblinEffectOut`, anything that starts with `gnoblin`). `naga` validates with no capability enabled, so there are no
//! storage buffers, atomics, compute or 16-bit floats.
//!
//! # The limits
//!
//! | limit | value | refused as |
//! | ----- | ----- | ---------- |
//! | source size | [`MAX_SHADER_SOURCE_BYTES`] (16 KiB) | too large, no position |
//! | loops | only `for (var i = START; i < END; i++)` (or `<=`, or `i += STEP`, or `i = i + STEP`) with constant `START`, `END` and a positive `STEP`, and at most [`MAX_LOOP_TRIPS`] (64) trips; `loop` and `while` are refused | at the loop |
//! | nested loops | at most [`MAX_LOOP_NEST`] (256) trips multiplied together, calls included | at the outer loop, where the product passes it |
//! | texture samples | at most [`MAX_TEXTURE_SAMPLES`] (16) reads in the worst case of `effect`, a `for` body counted once per trip | at `effect` |
//! | expressions | at most [`MAX_EXPRESSIONS`] (2048) in the shell's functions | at the function that passes it |
//! | recursion | none (`naga` refuses it) | where `naga` says |
//!
//! A loop is accepted by what it does, not by what it is called: its counter is written only by the header's update, so the
//! trip count is known before the program runs.
//!
//! The limits bound the program; the cost of running it is bounded by the renderers' watchdog (section 37.2 of the design).

use std::collections::HashMap;

use naga::front::wgsl::Frontend;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{
    BinaryOperator, Block, Expression, Function, GlobalVariable, Handle, LocalVariable, Module, ScalarKind, Span,
    Statement, TypeInner,
};

use gnoblin_render::MAX_SHADER_SOURCE_BYTES;

use super::sources::{USER_PREFIX, USER_SUFFIX};
use super::{ENTRY, ShaderError, Stage};

/// The most iterations one `for` loop of a shader may run.
pub const MAX_LOOP_TRIPS: u64 = 64;

/// The most iterations nested loops may run multiplied together, through calls too.
pub const MAX_LOOP_NEST: u64 = 256;

/// The most texture reads, `backdrop` calls included, `effect` may make in its worst case.
pub const MAX_TEXTURE_SAMPLES: u64 = 16;

/// The most expressions the shell's functions may hold between them.
pub const MAX_EXPRESSIONS: usize = 2048;

/// The pass name a refusal carries: `the shader pass line 4, column 9: ...`.
const PASS: &str = "shader";

/// The function the source must define.
pub const EFFECT_SIGNATURE: &str = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32>";

/// The names of the wrapper's own variables: the only variables a module may hold.
const WRAPPER_GLOBALS: [&str; 6] = [
    "gnoblin",
    "gnoblin_user",
    "gnoblin_tex",
    "gnoblin_sampler",
    "gnoblin_backdrop_tex",
    "gnoblin_backdrop_sampler",
];

/// The shape a loop must have, as the refusal of any other says.
const LOOP_SHAPE: &str = "a loop must be `for (var i = 0; i < 8; i++)`: a constant start, a `<` or `<=` bound that is a constant, a positive constant step, and a counter the body does not write";

/// What a source that passed is: the module a renderer compiles, and what the validator learned about it.
#[derive(Clone, Debug)]
pub struct Validated {
    wgsl: String,
    backdrop: bool,
    samples: u32,
    expressions: usize,
}

impl Validated {
    /// The whole fragment module: the shared preamble, the wrapper's block and `backdrop`, the shell's source, `shade` and the
    /// entry point [`ENTRY`]. A renderer compiles this and nothing else.
    #[must_use]
    pub fn wgsl(&self) -> &str {
        &self.wgsl
    }

    /// Whether `effect` can read the backdrop, so the renderer builds one for the layer.
    #[must_use]
    pub fn backdrop(&self) -> bool {
        self.backdrop
    }

    /// The most texture reads `effect` makes in its worst case.
    #[must_use]
    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// The expressions the shell's functions hold.
    #[must_use]
    pub fn expressions(&self) -> usize {
        self.expressions
    }
}

/// Checks `source` and returns the program, or the first thing wrong with it, at the line and column of the shell's own text.
///
/// # Errors
///
/// A [`ShaderError`] whose line and column count the shell's source (the first line is 1), or are 0 when the refusal has no
/// place in it: a source that is empty or too long, or a missing or mistyped `effect`.
pub fn validate(source: &str) -> Result<Validated, ShaderError> {
    if source.trim().is_empty() {
        return Err(refuse(0, 0, "the source is empty".to_owned()));
    }
    if source.len() > MAX_SHADER_SOURCE_BYTES {
        return Err(refuse(
            0,
            0,
            format!(
                "the source is {} bytes; the most is {MAX_SHADER_SOURCE_BYTES}",
                source.len()
            ),
        ));
    }
    let text = Text {
        source,
        start: USER_PREFIX.len(),
    };
    // The newline keeps a `//` comment on the shell's last line from reaching the suffix.
    let wgsl = format!("{USER_PREFIX}{source}\n{USER_SUFFIX}");
    let module = Frontend::new().parse(&wgsl).map_err(|error| {
        let at = error.labels().find_map(|(span, _)| text.locate(span));
        text.refusal(at, error.message().to_owned())
    })?;
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|error| {
            let at = error.spans().find_map(|context| text.locate(context.0));
            text.refusal(at, describe(error.as_inner()))
        })?;
    let facts = Inspect::new(&module, &text).run()?;
    Ok(Validated {
        wgsl,
        backdrop: facts.backdrop,
        samples: u32::try_from(facts.samples).unwrap_or(u32::MAX),
        expressions: facts.expressions,
    })
}

fn refuse(line: usize, column: usize, detail: String) -> ShaderError {
    ShaderError::new(PASS, Stage::Module, line, column, detail)
}

/// An error and the errors it was caused by, from the outermost in: `naga` keeps the reason in the chain.
fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        text.push_str(": ");
        text.push_str(&next.to_string());
        cause = next.source();
    }
    text
}

/// The shell's source and where it sits in the module `naga` was given.
struct Text<'a> {
    source: &'a str,
    /// The byte offset of the first byte of the source in the module.
    start: usize,
}

impl Text<'_> {
    /// The line and column in the shell's source of `span`, or [`None`] when the span lies in the wrapper.
    fn locate(&self, span: Span) -> Option<(usize, usize)> {
        let range = span.to_range()?;
        let at = range.start.checked_sub(self.start)?;
        if at >= self.source.len() {
            return None;
        }
        let before = self.source.get(..at)?;
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().map_or(0, |last| last.chars().count()) + 1;
        Some((line, column))
    }

    /// The refusal for something `naga` reported at `at`. A refusal that lies in the wrapper, a missing `effect` or a name
    /// clash, has no place in the shell's text and says what the source has to be.
    fn refusal(&self, at: Option<(usize, usize)>, detail: String) -> ShaderError {
        match at {
            Some((line, column)) => refuse(line, column, detail),
            None => refuse(
                0,
                0,
                format!(
                    "{detail} (the source must define `{EFFECT_SIGNATURE}`, and must not declare `shade`, `backdrop`, `GnoblinEffectOut` or a name that starts with `gnoblin`)"
                ),
            ),
        }
    }

    /// A refusal of something at `span`, which has a place only when the span lies in the shell's text.
    fn at(&self, span: Span, detail: impl Into<String>) -> ShaderError {
        let (line, column) = self.locate(span).unwrap_or((0, 0));
        refuse(line, column, detail.into())
    }
}

/// What walking a function found, worst case.
#[derive(Clone, Copy, Debug)]
struct Cost {
    /// Texture reads.
    samples: u64,
    /// The trips of the loops nested deepest, multiplied; 1 when there is no loop.
    nest: u64,
    /// Whether the function can read the backdrop texture.
    backdrop: bool,
}

impl Cost {
    const NONE: Self = Self {
        samples: 0,
        nest: 1,
        backdrop: false,
    };

    /// `other` runs after what `self` holds.
    fn then(&mut self, other: Self) {
        self.samples = self.samples.saturating_add(other.samples);
        self.nest = self.nest.max(other.nest);
        self.backdrop |= other.backdrop;
    }

    /// One of `self` or `other` runs.
    fn or(self, other: Self) -> Self {
        Self {
            samples: self.samples.max(other.samples),
            nest: self.nest.max(other.nest),
            backdrop: self.backdrop || other.backdrop,
        }
    }
}

/// What the whole module came to.
struct Findings {
    samples: u64,
    expressions: usize,
    backdrop: bool,
}

/// The walk over the validated module: declarations, loops, samples, expressions, the backdrop.
struct Inspect<'a> {
    module: &'a Module,
    text: &'a Text<'a>,
    memo: HashMap<Handle<Function>, Cost>,
    backdrop_texture: Option<Handle<GlobalVariable>>,
}

impl<'a> Inspect<'a> {
    fn new(module: &'a Module, text: &'a Text<'a>) -> Self {
        let backdrop_texture = module
            .global_variables
            .iter()
            .find(|(_, global)| global.name.as_deref() == Some("gnoblin_backdrop_tex"))
            .map(|(handle, _)| handle);
        Self {
            module,
            text,
            memo: HashMap::new(),
            backdrop_texture,
        }
    }

    fn run(mut self) -> Result<Findings, ShaderError> {
        let module = self.module;
        for (handle, global) in module.global_variables.iter() {
            let name = global.name.as_deref().unwrap_or("");
            if !WRAPPER_GLOBALS.contains(&name) {
                return Err(self.text.at(
                    module.global_variables.get_span(handle),
                    "a shader may not declare a variable, a texture or a buffer: the wrapper owns every binding",
                ));
            }
        }
        if let Some((handle, _)) = module.overrides.iter().next() {
            return Err(self.text.at(
                module.overrides.get_span(handle),
                "a shader may not declare an `override`",
            ));
        }
        for entry in &module.entry_points {
            if entry.name != ENTRY {
                return Err(self.text.at(
                    first_span(&entry.function),
                    "a shader may not declare an entry point: write `fn effect`",
                ));
            }
        }

        let mut expressions = 0usize;
        let mut effect = None;
        for (handle, function) in module.functions.iter() {
            let span = module.functions.get_span(handle);
            if self.text.locate(span).is_none() {
                continue;
            }
            if function.name.as_deref() == Some("effect") {
                effect = Some(handle);
            }
            let pointer = function.arguments.iter().any(|argument| {
                matches!(
                    module.types[argument.ty].inner,
                    TypeInner::Pointer { .. } | TypeInner::ValuePointer { .. }
                )
            });
            if pointer {
                return Err(self.text.at(span, "a function of a shader may not take a pointer"));
            }
            expressions += function.expressions.iter().len();
            if expressions > MAX_EXPRESSIONS {
                return Err(self.text.at(
                    span,
                    format!("the functions hold more than {MAX_EXPRESSIONS} expressions"),
                ));
            }
            // Every loop of every function is checked, whether `effect` reaches it or not.
            self.function_cost(handle)?;
        }
        let Some(effect) = effect else {
            return Err(self.text.refusal(None, "no function `effect`".to_owned()));
        };
        let cost = self.function_cost(effect)?;
        if cost.samples > MAX_TEXTURE_SAMPLES {
            return Err(self.text.at(
                module.functions.get_span(effect),
                format!(
                    "`effect` reads a texture up to {} times; the most is {MAX_TEXTURE_SAMPLES}",
                    cost.samples
                ),
            ));
        }
        Ok(Findings {
            samples: cost.samples,
            expressions,
            backdrop: cost.backdrop,
        })
    }

    fn function_cost(&mut self, handle: Handle<Function>) -> Result<Cost, ShaderError> {
        if let Some(cost) = self.memo.get(&handle) {
            return Ok(*cost);
        }
        let module = self.module;
        let function = &module.functions[handle];
        let mut cost = self.block_cost(function, &function.body)?;
        cost.backdrop |= function.expressions.iter().any(|(_, expression)| {
            matches!(expression, Expression::GlobalVariable(global) if Some(*global) == self.backdrop_texture)
        });
        self.memo.insert(handle, cost);
        Ok(cost)
    }

    fn block_cost(&mut self, function: &Function, block: &Block) -> Result<Cost, ShaderError> {
        let statements: &[Statement] = block;
        let mut total = Cost::NONE;
        for (index, (statement, span)) in block.span_iter().enumerate() {
            match statement {
                Statement::Emit(range) => {
                    let reads = range
                        .clone()
                        .filter(|handle| {
                            matches!(
                                function.expressions[*handle],
                                Expression::ImageSample { .. } | Expression::ImageLoad { .. }
                            )
                        })
                        .count() as u64;
                    total.samples = total.samples.saturating_add(reads);
                }
                Statement::Block(inner) => total.then(self.block_cost(function, inner)?),
                Statement::If { accept, reject, .. } => {
                    let accepted = self.block_cost(function, accept)?;
                    let rejected = self.block_cost(function, reject)?;
                    total.then(accepted.or(rejected));
                }
                Statement::Switch { cases, .. } => {
                    let mut worst = Cost::NONE;
                    for case in cases {
                        worst = worst.or(self.block_cost(function, &case.body)?);
                    }
                    total.then(worst);
                }
                Statement::Loop {
                    body,
                    continuing,
                    break_if,
                } => {
                    let trips = self
                        .loop_trips(function, &statements[..index], body, continuing, *break_if)
                        .map_err(|why| self.text.at(*span, why))?;
                    let mut inner = self.block_cost(function, body)?;
                    inner.then(self.block_cost(function, continuing)?);
                    let nest = trips.max(1).saturating_mul(inner.nest);
                    if nest > MAX_LOOP_NEST {
                        return Err(self.text.at(
                            *span,
                            format!("nested loops run {nest} times; the most is {MAX_LOOP_NEST}"),
                        ));
                    }
                    total.then(Cost {
                        samples: inner.samples.saturating_mul(trips),
                        nest,
                        backdrop: inner.backdrop,
                    });
                }
                Statement::Call { function: callee, .. } => {
                    let callee = self.function_cost(*callee)?;
                    total.then(callee);
                }
                _ => {}
            }
        }
        Ok(total)
    }

    /// How many times a loop runs, if it is the bounded shape.
    ///
    /// The `for` of WGSL arrives as the counter's initial store, then a `Loop` whose body opens with
    /// `if cond {} else { break }` and whose `continuing` block is the update. A loop is accepted when that shape holds with
    /// a counter nothing else writes, which is what makes the count a fact and not a hope.
    fn loop_trips(
        &self,
        function: &Function,
        preceding: &[Statement],
        body: &Block,
        continuing: &Block,
        break_if: Option<Handle<Expression>>,
    ) -> Result<u64, String> {
        let shape = || LOOP_SHAPE.to_owned();
        if break_if.is_some() {
            return Err(shape());
        }
        // The update: one store of `counter + step`.
        let mut updates = continuing
            .iter()
            .filter(|statement| !matches!(statement, Statement::Emit(_)));
        let (Some(Statement::Store { pointer, value }), None) = (updates.next(), updates.next()) else {
            return Err(shape());
        };
        let counter = local_of(function, *pointer).ok_or_else(shape)?;
        let Expression::Binary {
            op: BinaryOperator::Add,
            left,
            right,
        } = &function.expressions[*value]
        else {
            return Err(shape());
        };
        let reads_counter = |handle: Handle<Expression>| matches!(&function.expressions[handle], Expression::Load { pointer } if local_of(function, *pointer) == Some(counter));
        let step = if reads_counter(*left) {
            self.constant(function, *right)
        } else if reads_counter(*right) {
            self.constant(function, *left)
        } else {
            None
        };
        let step = step.filter(|step| *step > 0).ok_or_else(shape)?;

        // The condition: the first thing the body does.
        let first = body
            .iter()
            .find(|statement| !matches!(statement, Statement::Emit(_)))
            .ok_or_else(shape)?;
        let Statement::If {
            condition,
            accept,
            reject,
        } = first
        else {
            return Err(shape());
        };
        if !accept.is_empty() || reject.len() != 1 || !matches!(reject[0], Statement::Break) {
            return Err(shape());
        }
        let Expression::Binary { op, left, right } = &function.expressions[*condition] else {
            return Err(shape());
        };
        let inclusive = match op {
            BinaryOperator::Less => false,
            BinaryOperator::LessEqual => true,
            _ => return Err(shape()),
        };
        if !reads_counter(*left) {
            return Err(shape());
        }
        let bound = self.constant(function, *right).ok_or_else(shape)?;

        // The start: the store right before the loop, or the counter's own initial value when nothing else writes it.
        let own = &function.local_variables[counter];
        let before = preceding
            .iter()
            .rev()
            .find(|statement| !matches!(statement, Statement::Emit(_)));
        let start = match before {
            Some(Statement::Store { pointer, value }) if local_of(function, *pointer) == Some(counter) => {
                if count_stores(function, body, counter) != 0 {
                    return Err(shape());
                }
                self.constant(function, *value).ok_or_else(shape)?
            }
            _ => {
                if count_stores(function, &function.body, counter) != 1 {
                    return Err(shape());
                }
                match own.init {
                    Some(init) => self.constant(function, init).ok_or_else(shape)?,
                    None => 0,
                }
            }
        };
        let limit = match self.module.types[own.ty].inner {
            TypeInner::Scalar(scalar) if scalar.width == 4 && scalar.kind == ScalarKind::Sint => i64::from(i32::MAX),
            TypeInner::Scalar(scalar) if scalar.width == 4 && scalar.kind == ScalarKind::Uint => i64::from(u32::MAX),
            _ => return Err(shape()),
        };

        let trips = if inclusive {
            if bound < start { 0 } else { (bound - start) / step + 1 }
        } else if bound <= start {
            0
        } else {
            (bound - start + step - 1) / step
        };
        let trips = u64::try_from(trips).map_err(|_| shape())?;
        if trips > MAX_LOOP_TRIPS {
            return Err(format!("the loop runs {trips} times; the most is {MAX_LOOP_TRIPS}"));
        }
        // The counter must not wrap past its type on the way out, or the condition could hold for ever.
        let last = i64::try_from(trips)
            .map_err(|_| shape())?
            .saturating_mul(step)
            .saturating_add(start);
        if last > limit {
            return Err("the loop counter would pass the largest value its type holds".to_owned());
        }
        Ok(trips)
    }

    /// The integer a constant expression of the function is, if it is a literal or a named constant of one.
    fn constant(&self, function: &Function, handle: Handle<Expression>) -> Option<i64> {
        match &function.expressions[handle] {
            Expression::Literal(literal) => literal_int(literal),
            Expression::Constant(constant) => self.module_constant(self.module.constants[*constant].init),
            _ => None,
        }
    }

    fn module_constant(&self, handle: Handle<Expression>) -> Option<i64> {
        match &self.module.global_expressions[handle] {
            Expression::Literal(literal) => literal_int(literal),
            Expression::Constant(constant) => self.module_constant(self.module.constants[*constant].init),
            _ => None,
        }
    }
}

fn literal_int(literal: &naga::Literal) -> Option<i64> {
    match literal {
        naga::Literal::I32(value) => Some(i64::from(*value)),
        naga::Literal::U32(value) => Some(i64::from(*value)),
        naga::Literal::AbstractInt(value) => Some(*value),
        _ => None,
    }
}

/// The local variable an expression is the address of.
fn local_of(function: &Function, handle: Handle<Expression>) -> Option<Handle<LocalVariable>> {
    match &function.expressions[handle] {
        Expression::LocalVariable(variable) => Some(*variable),
        _ => None,
    }
}

/// How many stores to `variable` the statements of `block` hold, nested blocks and loops included.
fn count_stores(function: &Function, block: &Block, variable: Handle<LocalVariable>) -> usize {
    block
        .iter()
        .map(|statement| match statement {
            Statement::Store { pointer, .. } => usize::from(local_of(function, *pointer) == Some(variable)),
            Statement::Block(inner) => count_stores(function, inner, variable),
            Statement::If { accept, reject, .. } => {
                count_stores(function, accept, variable) + count_stores(function, reject, variable)
            }
            Statement::Switch { cases, .. } => cases
                .iter()
                .map(|case| count_stores(function, &case.body, variable))
                .sum(),
            Statement::Loop { body, continuing, .. } => {
                count_stores(function, body, variable) + count_stores(function, continuing, variable)
            }
            _ => 0,
        })
        .sum()
}

/// The first defined span of a function's expressions, which is as close to a position as an entry point has.
fn first_span(function: &Function) -> Span {
    function
        .expressions
        .iter()
        .map(|(handle, _)| function.expressions.get_span(handle))
        .find(|span| span.is_defined())
        .unwrap_or(Span::UNDEFINED)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {\n";

    /// A source whose `effect` runs `body`.
    fn effect(body: &str) -> String {
        format!("{HEAD}{body}\n}}\n")
    }

    fn accepted(source: &str) -> Validated {
        match validate(source) {
            Ok(validated) => validated,
            Err(error) => panic!("the source should pass, got {error}\n{source}"),
        }
    }

    fn refused(source: &str) -> ShaderError {
        match validate(source) {
            Ok(_) => panic!("the source should be refused:\n{source}"),
            Err(error) => error,
        }
    }

    #[test]
    fn a_tint_that_reads_params_and_time_passes() {
        let tint = accepted(&effect("return vec4<f32>(c.rgb * p[0], c.a) * (1.0 + sin(t));"));
        assert!(!tint.backdrop(), "it never reads the backdrop");
        assert_eq!(tint.samples(), 0);
        assert!(
            tint.wgsl().contains("fn gnoblin_entry("),
            "the module ends in the shared entry point"
        );
    }

    #[test]
    fn constants_helpers_and_structs_are_allowed() {
        let source = "\
const TAPS = 4;
struct Wave { amplitude: f32, speed: f32 }
fn wave(w: Wave, x: f32, t: f32) -> f32 { return w.amplitude * sin(x * 6.28 + t * w.speed); }
fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
    let w = Wave(p[0], p[1]);
    var acc = 0.0;
    for (var i = 0; i < TAPS; i++) { acc = acc + wave(w, uv.x, t); }
    return c * (1.0 + acc);
}
";
        accepted(source);
    }

    #[test]
    fn a_shader_that_calls_backdrop_says_so_and_counts_its_samples() {
        let blur = accepted(&effect(
            "var sum = vec4<f32>(0.0);\nfor (var i = 0; i < 9; i++) {\n    sum = sum + backdrop(uv + vec2<f32>(f32(i) * 0.01, 0.0));\n}\nreturn sum / 9.0;",
        ));
        assert!(blur.backdrop());
        assert_eq!(blur.samples(), 9, "one read in a loop of nine trips");
    }

    #[test]
    fn reaching_the_backdrop_through_a_helper_or_the_texture_itself_counts() {
        let helper = "fn look(uv: vec2<f32>) -> vec4<f32> { return backdrop(uv); }\n";
        assert!(accepted(&format!("{helper}{}", effect("return look(uv);"))).backdrop());
        let direct = effect("return textureSampleLevel(gnoblin_backdrop_tex, gnoblin_backdrop_sampler, uv, 0.0);");
        assert!(accepted(&direct).backdrop());
        let unused = format!("{helper}{}", effect("return c;"));
        assert!(!accepted(&unused).backdrop(), "a helper nobody calls reads nothing");
    }

    #[test]
    fn sixteen_texture_reads_pass_and_seventeen_do_not() {
        let reads = |trips: u32| {
            effect(&format!(
                "var s = vec4<f32>(0.0);\nfor (var i = 0; i < {trips}; i++) {{ s = s + backdrop(uv); }}\nreturn s;"
            ))
        };
        assert_eq!(accepted(&reads(16)).samples(), 16);
        let error = refused(&reads(17));
        assert!(error.detail().contains("17"), "{error}");
        assert!(error.detail().contains("16"), "{error}");
        assert_eq!(error.line(), 1, "the refusal is at `effect`");
    }

    #[test]
    fn a_branch_costs_its_worse_arm_not_both() {
        let source = effect(
            "var r = backdrop(uv);\nif p[0] > 0.5 {\n    r = r + backdrop(uv + vec2<f32>(0.1, 0.0)) + backdrop(uv + vec2<f32>(0.2, 0.0));\n} else {\n    r = r + backdrop(uv + vec2<f32>(0.3, 0.0));\n}\nreturn r;",
        );
        assert_eq!(
            accepted(&source).samples(),
            3,
            "one before the branch and the two of the longer arm"
        );
    }

    #[test]
    fn a_for_loop_runs_at_most_sixty_four_times() {
        let spin = |header: &str| {
            effect(&format!(
                "var a = 0.0;\nfor {header} {{ a = a + 1.0; }}\nreturn vec4<f32>(a);"
            ))
        };
        accepted(&spin("(var i = 0; i < 64; i++)"));
        accepted(&spin("(var i = 0; i <= 63; i++)"));
        accepted(&spin("(var i = 0; i < 128; i += 2)"));
        accepted(&spin("(var i = 10u; i < 20u; i = i + 1u)"));
        let error = refused(&spin("(var i = 0; i < 65; i++)"));
        assert!(error.detail().contains("65"), "{error}");
        assert_eq!((error.line(), error.column()), (3, 1), "the refusal is at the loop");
        refused(&spin("(var i = 0; i <= 64; i++)"));
        refused(&spin("(var i = 0; i < 1000; i += 2)"));
    }

    #[test]
    fn nested_loops_multiply() {
        let nest = |outer: u32, inner: u32| {
            effect(&format!(
                "var a = 0.0;\nfor (var i = 0; i < {outer}; i++) {{\n    for (var j = 0; j < {inner}; j++) {{ a = a + 1.0; }}\n}}\nreturn vec4<f32>(a);"
            ))
        };
        accepted(&nest(16, 16));
        let error = refused(&nest(17, 16));
        assert!(error.detail().contains("272"), "{error}");
        assert_eq!(
            error.line(),
            3,
            "the refusal is at the outer loop, where the product passes the ceiling"
        );
    }

    #[test]
    fn loop_and_while_are_refused() {
        let looped = refused(&effect(
            "var a = 0.0;\nloop {\n    a = a + 1.0;\n    if a > 3.0 { break; }\n}\nreturn vec4<f32>(a);",
        ));
        assert_eq!((looped.line(), looped.column()), (3, 1), "{looped}");
        assert!(
            looped.detail().contains("for (var i"),
            "the refusal says what is allowed: {looped}"
        );
        let spun = refused(&effect(
            "var a = 0.0;\nwhile a < 3.0 { a = a + 1.0; }\nreturn vec4<f32>(a);",
        ));
        assert_eq!(spun.line(), 3, "{spun}");
    }

    #[test]
    fn a_for_loop_whose_bound_is_not_a_constant_is_refused() {
        let error = refused(&effect(
            "let n = i32(p[0]);\nvar a = 0.0;\nfor (var i = 0; i < n; i++) { a = a + 1.0; }\nreturn vec4<f32>(a);",
        ));
        assert_eq!(error.line(), 4, "{error}");
        let start = refused(&effect(
            "var a = 0.0;\nfor (var i = i32(p[0]); i < 4; i++) { a = a + 1.0; }\nreturn vec4<f32>(a);",
        ));
        assert_eq!(start.line(), 3, "{start}");
    }

    #[test]
    fn a_for_loop_that_writes_its_own_counter_is_refused() {
        refused(&effect(
            "var a = 0.0;\nfor (var i = 0; i < 4; i++) {\n    i = 0;\n    a = a + 1.0;\n}\nreturn vec4<f32>(a);",
        ));
        refused(&effect(
            "var a = 0.0;\nfor (var i = 0; i < 4; i++) {\n    if a > 1.0 { i -= 1; }\n    a = a + 1.0;\n}\nreturn vec4<f32>(a);",
        ));
    }

    #[test]
    fn a_for_loop_that_does_not_count_up_is_refused() {
        let spin = |header: &str| {
            effect(&format!(
                "var a = 0.0;\nfor {header} {{ a = a + 1.0; }}\nreturn vec4<f32>(a);"
            ))
        };
        refused(&spin("(var i = 0; i < 4; )"));
        refused(&spin("(var i = 0; i < 4; i += 0)"));
        refused(&spin("(var i = 4; i > 0; i--)"));
        refused(&spin("(var i = 0.0; i < 4.0; i += 1.0)"));
    }

    #[test]
    fn a_counter_that_would_wrap_past_its_type_is_refused() {
        let error = refused(&effect(
            "var a = 0.0;\nfor (var i = 2147483640; i < 2147483647; i += 100000000) { a = a + 1.0; }\nreturn vec4<f32>(a);",
        ));
        assert!(error.detail().contains("largest value"), "{error}");
    }

    #[test]
    fn a_source_that_is_empty_or_too_large_is_refused_without_a_position() {
        let empty = refused("  \n");
        assert_eq!((empty.line(), empty.column()), (0, 0));
        let filler = "// x\n".repeat(MAX_SHADER_SOURCE_BYTES / 5 + 1);
        let big = refused(&format!("{}{filler}", effect("return c;")));
        assert_eq!(big.line(), 0);
        assert!(big.detail().contains("16384"), "{big}");
        let exactly = format!(
            "{}{}",
            effect("return c;"),
            "//".repeat((MAX_SHADER_SOURCE_BYTES - effect("return c;").len()) / 2)
        );
        assert!(exactly.len() <= MAX_SHADER_SOURCE_BYTES);
        accepted(&exactly);
    }

    #[test]
    fn the_expression_count_is_bounded() {
        let body = "a=a+c;\n".repeat(1200);
        let error = refused(&effect(&format!("var a = c;\n{body}return a;")));
        assert!(error.detail().contains("2048"), "{error}");
        assert_eq!(error.line(), 1);
    }

    #[test]
    fn a_syntax_error_names_the_line_and_column_of_the_shell_source() {
        let source = "\
fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
    let x = 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}
";
        let error = refused(source);
        assert_eq!((error.line(), error.column()), (3, 25), "{error}");
        assert!(error.to_string().contains("line 3, column 25"), "{error}");
    }

    #[test]
    fn a_type_error_names_the_line_of_the_expression() {
        let source = effect("let x: f32 = vec2<f32>(1.0, 2.0);\nreturn c;");
        let error = refused(&source);
        assert_eq!(error.line(), 2, "{error}");
    }

    #[test]
    fn a_missing_or_mistyped_effect_says_what_is_required() {
        let missing = refused("fn other() -> f32 { return 1.0; }\n");
        assert_eq!(missing.line(), 0, "there is no line to name");
        assert!(missing.detail().contains("fn effect("), "{missing}");
        let mistyped = refused("fn effect(c: vec4<f32>) -> vec4<f32> { return c; }\n");
        assert_eq!(mistyped.line(), 0);
        assert!(mistyped.detail().contains("fn effect("), "{mistyped}");
        let wrong_result =
            refused("fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> f32 { return 1.0; }\n");
        assert_eq!(wrong_result.line(), 0);
    }

    #[test]
    fn a_name_the_wrapper_owns_is_refused_at_the_declaration() {
        let source = format!(
            "fn backdrop(uv: vec2<f32>) -> vec4<f32> {{ return vec4<f32>(0.0); }}\n{}",
            effect("return c;")
        );
        let error = refused(&source);
        assert!(error.detail().contains("backdrop"), "{error}");
        let shade = refused(&format!("fn shade() {{}}\n{}", effect("return c;")));
        assert!(shade.detail().contains("shade"), "{shade}");
    }

    #[test]
    fn variables_textures_buffers_overrides_and_entry_points_are_refused() {
        let with = |declaration: &str| format!("{declaration}\n{}", effect("return c;"));
        for (declaration, what) in [
            ("var<private> counter: f32;", "variable"),
            ("var<workgroup> scratch: f32;", "variable"),
            ("@group(3) @binding(0) var<uniform> extra: vec4<f32>;", "variable"),
            ("@group(3) @binding(1) var extra_tex: texture_2d<f32>;", "variable"),
            (
                "@group(3) @binding(2) var<storage, read_write> sink: array<f32>;",
                "variable",
            ),
            ("override scale: f32 = 1.0;", "override"),
            (
                "@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }",
                "entry point",
            ),
            ("@compute @workgroup_size(1) fn main() {}", "entry point"),
        ] {
            let error = refused(&with(declaration));
            assert!(
                error.detail().contains(what),
                "`{declaration}` should be refused as a {what}: {error}"
            );
            if what == "variable" {
                assert_eq!(error.line(), 1, "at its declaration: {error}");
            }
        }
    }

    #[test]
    fn a_function_that_takes_a_pointer_is_refused() {
        let source = format!(
            "fn bump(x: ptr<function, f32>) {{ *x = 1.0; }}\n{}",
            effect("var a = 0.0;\nbump(&a);\nreturn vec4<f32>(a);")
        );
        let error = refused(&source);
        assert_eq!(error.line(), 1, "{error}");
        assert!(error.detail().contains("pointer"), "{error}");
    }

    #[test]
    fn recursion_is_refused() {
        let source = format!(
            "fn again(x: f32) -> f32 {{ return again(x); }}\n{}",
            effect("return vec4<f32>(again(c.r));")
        );
        let error = refused(&source);
        assert!(error.line() >= 1, "naga names the place: {error}");
    }

    #[test]
    fn sixteen_bit_floats_and_other_capabilities_are_refused() {
        refused(&effect("let h: f16 = 1.0h;\nreturn c * f32(h);"));
    }
}
