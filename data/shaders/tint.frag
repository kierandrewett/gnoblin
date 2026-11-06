// Configure shader-uniforms.strength between 0.0 and 1.0.
uniform float strength;

vec4 gnoblin_effect(vec4 color, vec2 uv) {
    return vec4(mix(color.rgb, vec3(0.24, 0.30, 0.40), clamp(strength, 0.0, 1.0)), color.a);
}
