struct SkinnedVertex {
    position: vec4<f32>,
    normal: vec4<f32>,
    uv: vec2<f32>,
    _padding: vec2<f32>,
    bone_indices: vec4<u32>,
    bone_weights: vec4<f32>,
};

struct MeshletDescription {
    vertex_list_offset: u32,
    triangle_list_offset: u32,
    triangle_count: u32,
    vertex_count: u32,
};

struct DrawCommand {
    meshlet_id: u32,
    transform_id: u32,
    texture_id: u32,
    _padding: u32,
};

struct VSOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) texture_id: u32,
};

//-- Bindings ------------------------------------------------------------------

@group(0) @binding(0) var<uniform> camera: mat4x4<f32>;
@group(0) @binding(1) var<storage, read> bone_matrices: array<mat4x4<f32>>;

@group(1) @binding(0) var<storage, read> vertices: array<SkinnedVertex>;
@group(1) @binding(1) var<storage, read> meshlet_vertex_indices: array<u32>;
@group(1) @binding(2) var<storage, read> meshlet_triangle_indices: array<u32>;
@group(1) @binding(3) var<storage, read> meshlet_descriptions: array<MeshletDescription>;

@group(2) @binding(0) var<storage, read> indirection_buffer: array<DrawCommand>;
@group(2) @binding(1) var<storage, read> transform_buffer: array<mat4x4<f32>>;

@group(3) @binding(0) var texture_array: texture_2d_array<f32>;
@group(3) @binding(1) var texture_sampler: sampler;

//-- Vertex Shader -------------------------------------------------------------

@vertex
fn vs_main(
    @builtin(instance_index) instance_id: u32,
    @builtin(vertex_index) local_vtx_id: u32
) -> VSOutput {
    var output: VSOutput;
    output.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    output.world_normal = vec3<f32>(0.0, 0.0, 0.0);
    output.uv = vec2<f32>(0.0, 0.0);
    output.texture_id = 0u;

    let command = indirection_buffer[instance_id];
    let meshlet = meshlet_descriptions[command.meshlet_id];

    if (local_vtx_id >= meshlet.triangle_count * 3u) {
        return output;
    }

    let total_byte_offset = meshlet.triangle_list_offset + local_vtx_id;
    let u32_index = total_byte_offset / 4u;
    let byte_in_u32 = total_byte_offset % 4u;
    let packed_indices = meshlet_triangle_indices[u32_index];
    let local_vertex_index_in_meshlet = (packed_indices >> (byte_in_u32 * 8u)) & 0xFFu;

    let vertex_index_in_list = meshlet.vertex_list_offset + local_vertex_index_in_meshlet;
    let final_vertex_index = meshlet_vertex_indices[vertex_index_in_list];

    let vertex = vertices[final_vertex_index];

    // Skinning
    var skin_transform = mat4x4<f32>();
    for (var i = 0; i < 4; i = i + 1) {
        let bone_index = vertex.bone_indices[i];
        let bone_weight = vertex.bone_weights[i];
        if (bone_weight > 0.0) {
            skin_transform = skin_transform + bone_matrices[bone_index] * bone_weight;
        }
    }

    let model_transform = transform_buffer[command.transform_id];
    let skinned_pos = skin_transform * vertex.position;
    let world_pos = model_transform * skinned_pos;

    output.clip_position = camera * world_pos;
    output.world_normal = normalize((model_transform * skin_transform * vec4<f32>(vertex.normal.xyz, 0.0)).xyz);
    output.uv = vertex.uv;
    output.texture_id = command.texture_id;

    return output;
}

//-- Fragment Shader -----------------------------------------------------------

@fragment
fn fs_main(in: VSOutput) -> @location(0) vec4<f32> {
    let base_color = textureSample(texture_array, texture_sampler, in.uv, in.texture_id);
    let final_color = base_color.rgb;
    return vec4<f32>(final_color, 1.0);
} 