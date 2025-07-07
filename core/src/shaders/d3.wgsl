// The structure of a single vertex, matching the Rust `Vertex` struct.
struct Vertex {
    position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
};

// Static asset data describing a slice of the geometry buffers.
// Matches the Rust `MeshletDescription` struct.
struct MeshletDescription {
    vertex_list_offset: u32,
    triangle_list_offset: u32,
    triangle_count: u32,
    _padding: u32,
};

// Dynamic per-frame command telling the GPU what to draw.
// Matches the Rust `DrawCommand` struct.
struct DrawCommand {
    meshlet_id: u32,
    transform_id: u32,
};

// Data passed from vertex to fragment stage.
struct VSOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};


//-- Bindings ------------------------------------------------------------------

// This group will contain camera data.
@group(0) @binding(0) var<uniform> camera: mat4x4<f32>;

// This group contains all the mesh and instance data.
// It matches the layout created in `assets.rs`.
@group(1) @binding(0) var<storage, read> vertices: array<Vertex>;
@group(1) @binding(1) var<storage, read> meshlet_vertex_indices: array<u32>;
@group(1) @binding(2) var<storage, read> meshlet_triangle_indices: array<u32>; // u8s packed into u32s
@group(1) @binding(3) var<storage, read> meshlet_descriptions: array<MeshletDescription>;
@group(1) @binding(4) var<storage, read> indirection_buffer: array<DrawCommand>;
@group(1) @binding(5) var<storage, read> transform_buffer: array<mat4x4<f32>>;


//-- Vertex Shader -------------------------------------------------------------

@vertex
fn vs_main(
    @builtin(instance_index) instance_id: u32, // The index of the draw command we're executing.
    @builtin(vertex_index) local_vtx_id: u32   // The index of the vertex within this meshlet's triangles.
) -> VSOutput {
    var output: VSOutput;

    // 1. Fetch the draw command for this instance.
    let command = indirection_buffer[instance_id];

    // 2. Use the command to get the meshlet's metadata.
    let meshlet = meshlet_descriptions[command.meshlet_id];

    // 3. Cull padded vertices. If the vertex index is beyond the actual number of
    //    indices in the meshlet, push it outside the visible clip space and exit early.
    //    The GPU will discard these vertices.
    if (local_vtx_id >= meshlet.triangle_count * 3u) {
        output.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return output;
    }

    // 4. Perform the two-level index lookup.
    
    // First, find the local `u8` index for this vertex. Since we packed four u8s into
    // each u32 on the CPU, we need to find which u32 contains our index and then
    // extract the correct byte from it.
    let packed_indices_offset = meshlet.triangle_list_offset / 4u;
    let index_in_u32 = local_vtx_id % 4u;
    let packed_indices = meshlet_triangle_indices[packed_indices_offset + (local_vtx_id / 4u)];
    let local_vertex_index_in_meshlet = (packed_indices >> (index_in_u32 * 8u)) & 0xFFu;

    // Second, use that local index to look up the global vertex index. This points
    // into the main `vertices` buffer.
    let vertex_index_in_list = meshlet.vertex_list_offset + local_vertex_index_in_meshlet;
    let final_vertex_index = meshlet_vertex_indices[vertex_index_in_list];

    // 5. Fetch the final vertex data using the resolved index.
    let vertex = vertices[final_vertex_index];

    // 6. Apply transformations using the transform_id from the draw command.
    let model_transform = transform_buffer[command.transform_id];
    let world_pos = model_transform * vec4<f32>(vertex.position, 1.0);

    output.clip_position = camera * world_pos;
    output.world_normal = normalize((model_transform * vec4<f32>(vertex.normal, 0.0)).xyz);
    output.uv = vertex.uv;

    return output;
}


//-- Fragment Shader -----------------------------------------------------------

@fragment
fn fs_main(in: VSOutput) -> @location(0) vec4<f32> {
    // Simple lighting model
    let light_dir = normalize(vec3<f32>(0.5, 1.0, 0.6));
    // Use the world normal passed from the vertex shader for lighting calculations.
    let diffuse_light = max(dot(in.world_normal, light_dir), 0.1);
    // For now, output a solid color combined with the diffuse light.
    // A real implementation would sample a texture using `in.uv`.
    let base_color = vec3<f32>(0.8, 0.7, 0.6);
    let final_color = base_color * diffuse_light;
    return vec4<f32>(final_color, 1.0);
} 