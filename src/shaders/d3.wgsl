struct Vertex {
    position: vec4<f32>,
};

struct ViewProjection {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0)
var<uniform> u_view_proj: ViewProjection;

struct MeshInfo {
    transform: mat4x4<f32>,
    index_count: u32,
    first_index: u32,
    base_vertex: u32,
    _padding: u32,
};

@group(1) @binding(0)
var<storage, read> vertices: array<Vertex>;
@group(1) @binding(1)
var<storage, read> indices: array<u32>;
@group(1) @binding(2)
var<storage, read> mesh_infos: array<MeshInfo>;
@group(1) @binding(3)
var<storage, read> draw_index_to_mesh_id: array<u32>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    let mesh_id = draw_index_to_mesh_id[in_vertex_index];
    let mesh_info = mesh_infos[mesh_id];

    let local_draw_index = in_vertex_index - mesh_info.first_index;
    let index_location = mesh_info.first_index + local_draw_index;
    let local_vertex_index = indices[index_location];
    
    let final_vertex_index = mesh_info.base_vertex + local_vertex_index;

    let vertex = vertices[final_vertex_index];

    var out: VertexOutput;
    out.clip_position = u_view_proj.view_proj * mesh_info.transform * vertex.position;
    out.color = vertex.position.xyz * 0.5 + 0.5;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
} 