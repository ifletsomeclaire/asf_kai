struct Locals {
    angle: f32,
};

@group(0) @binding(0)
var<uniform> u_locals: Locals;

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> @builtin(position) vec4<f32> {
    var vertices = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 0.5),
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, -0.5)
    );

    let angle = u_locals.angle;
    let cos_angle = cos(angle);
    let sin_angle = sin(angle);

    let x = vertices[in_vertex_index].x * cos_angle - vertices[in_vertex_index].y * sin_angle;
    let y = vertices[in_vertex_index].x * sin_angle + vertices[in_vertex_index].y * cos_angle;

    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.5, 0.0, 1.0); // Orange
} 