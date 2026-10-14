#version 450
// A unit quad centred on the origin, drawn as a triangle strip. The model matrix places and
// sizes it in the scene.
layout(push_constant) uniform Push {
    mat4 mvp;
} push;
layout(location = 0) out vec2 uv;

void main() {
    vec2 corner = vec2(float(gl_VertexIndex & 1), float(gl_VertexIndex >> 1));
    uv = corner;
    gl_Position = push.mvp * vec4(corner.x - 0.5, 0.5 - corner.y, 0.0, 1.0);
}

