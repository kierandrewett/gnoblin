//! The WGSL sources of the passes, as the compiler receives them.
//!
//! Every fragment stage is the preamble plus a body plus an entry point, concatenated at
//! compile time so the shipped passes and a user's shader go through exactly one path: a
//! pass that compiles here compiles for a user, and a body that would not would fail here
//! first.
//!
//! The preamble's struct ends with a marker line that [`preamble`] replaces with a schema's
//! fields, which is what makes a user's uniforms members of the same block as the
//! framework's. The marker is a comment, so the effect passes — which declare no user
//! fields — compile with it left in place.

/// The uniform block, the texture pair and the helpers every fragment stage sees.
const PREAMBLE: &str = include_str!("shaders/preamble.wgsl");

/// The line a schema's fields replace.
const USER_FIELDS: &str = "    // gnoblin:user-fields";

/// The vertex stage every pass shares.
pub(crate) const VERTEX: &str = include_str!("shaders/quad.wgsl");

/// A rectangle of one colour, with the layer's corner radius and opacity.
pub(crate) const SOLID: &str = concat!(
    include_str!("shaders/preamble.wgsl"),
    include_str!("shaders/solid.wgsl"),
    include_str!("shaders/effect.wgsl"),
);

/// A layer's own pixels: cropped, scaled, tinted and clipped to its corner radius.
pub(crate) const SURFACE: &str = concat!(
    include_str!("shaders/preamble.wgsl"),
    include_str!("shaders/surface.wgsl"),
    include_str!("shaders/effect.wgsl"),
);

/// A drop shadow behind a layer.
pub(crate) const SHADOW: &str = concat!(
    include_str!("shaders/preamble.wgsl"),
    include_str!("shaders/shadow.wgsl"),
    include_str!("shaders/effect.wgsl"),
);

/// One axis of the Gaussian blur of the backdrop behind a layer.
pub(crate) const BLUR: &str = concat!(
    include_str!("shaders/preamble.wgsl"),
    include_str!("shaders/blur.wgsl"),
    include_str!("shaders/effect.wgsl"),
);

/// The entry point a user shader is compiled with.
pub(crate) const USER_ENTRY: &str = include_str!("shaders/user.wgsl");

/// The preamble with a schema's uniforms declared in the uniform block.
///
/// The fields are appended after the framework's, in declaration order, which is the order
/// [`super::program::UniformLayout`] packs them in.
pub(crate) fn preamble(schema: &gnoblin_render::UniformSchema) -> String {
    let mut fields = String::new();
    for (name, kind) in schema.entries() {
        fields.push_str("    ");
        fields.push_str(name);
        fields.push_str(": ");
        fields.push_str(type_spelling(*kind));
        fields.push(',');
        fields.push('\n');
    }
    PREAMBLE.replace(USER_FIELDS, fields.trim_end_matches('\n'))
}

/// The WGSL spelling of a declared uniform type, which is what the schema's type names mean.
fn type_spelling(kind: gnoblin_render::UniformType) -> &'static str {
    use gnoblin_render::UniformType;
    match kind {
        UniformType::Float => "f32",
        UniformType::Vec2 => "vec2<f32>",
        UniformType::Vec3 => "vec3<f32>",
        UniformType::Vec4 => "vec4<f32>",
        UniformType::Int => "i32",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_render::UniformType;

    #[test]
    fn a_schema_becomes_fields_of_the_uniform_block() {
        let schema =
            gnoblin_render::UniformSchema::new(vec![("u_tint", UniformType::Vec4), ("u_amount", UniformType::Float)])
                .expect("a schema");
        let preamble = preamble(&schema);
        assert!(
            preamble.contains("    u_tint: vec4<f32>,\n    u_amount: f32,\n"),
            "the declarations are fields of the block, in order: {preamble}"
        );
        assert!(!preamble.contains(USER_FIELDS), "the marker is gone");
    }

    #[test]
    fn an_empty_schema_leaves_the_block_alone() {
        let preamble = preamble(&gnoblin_render::UniformSchema::default());
        assert!(!preamble.contains(USER_FIELDS));
        assert!(preamble.contains("padding: vec2<f32>,"), "the fixed members survive");
    }

    #[test]
    fn every_pass_declares_the_entry_point() {
        for pass in [SOLID, SURFACE, SHADOW, BLUR] {
            assert!(pass.contains("fn shade("), "a pass defines its body");
            assert!(pass.contains("fn gnoblin_entry("), "and the shared entry point");
        }
        assert!(VERTEX.contains("fn gnoblin_entry("));
        assert!(USER_ENTRY.contains("fn gnoblin_entry("));
    }
}
