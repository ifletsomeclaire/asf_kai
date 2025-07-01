@group(0) @binding(0)
var<uniform> u_angle: f32;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    let angle = u_angle;
    let cos = cos(angle);
    let sin = sin(angle);
    let rot = mat2x2<f32>(
        cos, -sin,
        sin, cos,
    );

    var vertices = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 0.5),
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, -0.5)
    );
    let p = rot * vertices[in_vertex_index];

    var colors = array<vec3<f32>, 3>(
        vec3(1.0, 0.0, 0.0),
        vec3(0.0, 1.0, 0.0),
        vec3(0.0, 0.0, 1.0),
    );

    var out: VertexOutput;
    out.clip_position = vec4<f32>(p, 0.0, 1.0);
    out.color = colors[in_vertex_index];
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
} 