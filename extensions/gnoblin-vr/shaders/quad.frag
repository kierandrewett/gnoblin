#version 450
layout(set = 0, binding = 0) uniform sampler2D window;
layout(push_constant) uniform Push {
    layout(offset = 64) vec4 shape; // width and height in metres, corner radius in metres, opacity
} push;
layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 colour;

// Signed distance to a rounded rectangle centred on the origin; negative inside.
float rounded_box(vec2 p, vec2 half_size, float radius) {
    vec2 q = abs(p) - half_size + radius;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

void main() {
    vec2 size = push.shape.xy;
    float radius = min(push.shape.z, 0.5 * min(size.x, size.y));
    float distance_to_edge = rounded_box((uv - 0.5) * size, 0.5 * size, radius);
    // One pixel of smoothing, measured in metres, so edges stay clean at any distance.
    float coverage = clamp(0.5 - distance_to_edge / max(fwidth(distance_to_edge), 1e-6), 0.0, 1.0);
    // The frame is premultiplied BGRA, so everything scales together.
    colour = texture(window, uv) * (coverage * push.shape.w);
}

