// GPU Data Layout
// ------------------------------------------------------------------

struct Camera {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

struct Vertex {
    position: vec4<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
};
struct Index {
    i: u32,
};
struct MeshDescription {
    index_count: u32,
    first_index: u32,
    base_vertex: i32,
};
struct Instance {
    model_matrix: mat4x4<f32>,
    mesh_id: u32,
    texture_array_index: u32,
};
struct InstanceLookup {
    instance_id: u32,
    local_vertex_index: u32,
};

@group(1) @binding(0)
var<storage, read> vertices: array<Vertex>;
@group(1) @binding(1)
var<storage, read> indices: array<Index>;
@group(1) @binding(2)
var<storage, read> mesh_descriptions: array<MeshDescription>;
@group(1) @binding(3)
var<storage, read> instances: array<Instance>;
@group(1) @binding(4)
var<storage, read> instance_lookups: array<InstanceLookup>;
@group(1) @binding(5)
var texture_atlas: texture_2d_array<f32>;
@group(1) @binding(6)
var texture_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) texture_array_index: u32,
};

// Shader Entry Point
// ------------------------------------------------------------------

@vertex
fn vs_main(
    @builtin(vertex_index) in_vertex_index: u32,
) -> VertexOutput {
    let lookup = instance_lookups[in_vertex_index];
    let instance = instances[lookup.instance_id];
    let mesh = mesh_descriptions[instance.mesh_id];

    let local_vertex_index = lookup.local_vertex_index;
    let index_in_global_buffer = mesh.first_index + local_vertex_index;
    let final_vertex_index = u32(mesh.base_vertex) + indices[index_in_global_buffer].i;
    let vertex = vertices[final_vertex_index];

    let world_position = instance.model_matrix * vertex.position;

    var out: VertexOutput;
    out.clip_position = camera.view_proj * world_position;
    out.uv = vertex.uv;
    out.texture_array_index = instance.texture_array_index;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if (in.texture_array_index >= 9999u) {
        return vec4<f32>(1.0, 0.0, 1.0, 1.0); // Magenta for unloaded texture
    }
    return textureSample(texture_atlas, texture_sampler, in.uv, in.texture_array_index);
} 