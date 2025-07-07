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
    vertex_count: u32,
};

// Dynamic per-frame command telling the GPU what to draw.
// Matches the Rust `DrawCommand` struct.
struct DrawCommand {
    meshlet_id: u32,
    transform_id: u32,
    texture_id: u32,
    _padding: u32,
};

// Data passed from vertex to fragment stage.
struct VSOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) texture_id: u32,
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
@group(1) @binding(6) var texture_array: texture_2d_array<f32>;
@group(1) @binding(7) var texture_sampler: sampler;


//-- Vertex Shader -------------------------------------------------------------

@vertex
fn vs_main(
    @builtin(instance_index) instance_id: u32, // The index of the draw command we're executing.
    @builtin(vertex_index) local_vtx_id: u32   // The index of the vertex within this meshlet's triangles.
) -> VSOutput {
    // Initialize output to default values to satisfy WGSL's requirement that
    // all `var`s must be initialized before a function returns. This prevents
    // undefined behavior from returning a partially-initialized struct.
    var output: VSOutput;
    output.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0); // Outside clip space
    output.world_normal = vec3<f32>(0.0, 0.0, 0.0);
    output.uv = vec2<f32>(0.0, 0.0);
    output.texture_id = 0u;

    // 1. Fetch the draw command for this instance.
    let command = indirection_buffer[instance_id];

    // 2. Use the command to get the meshlet's metadata.
    let meshlet = meshlet_descriptions[command.meshlet_id];

    // 3. Cull padded vertices. If the vertex index is beyond the actual number of
    //    indices in the meshlet, return the default "outside clip space" vertex.
    if (local_vtx_id >= meshlet.triangle_count * 3u) {
        return output;
    }

    // 4. Perform the two-level index lookup.
    
    // First, calculate the absolute byte offset for the vertex index we need.
    let total_byte_offset = meshlet.triangle_list_offset + local_vtx_id;

    // Then, use that absolute offset to find the correct u32 in the packed buffer
    // and the correct byte within that u32.
    let u32_index = total_byte_offset / 4u;
    let byte_in_u32 = total_byte_offset % 4u;
    let packed_indices = meshlet_triangle_indices[u32_index];
    let local_vertex_index_in_meshlet = (packed_indices >> (byte_in_u32 * 8u)) & 0xFFu;

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
    output.texture_id = command.texture_id;

    return output;
}


//-- Fragment Shader -----------------------------------------------------------

@fragment
fn fs_main(in: VSOutput) -> @location(0) vec4<f32> {

    // Sample the texture
    let base_color = textureSample(texture_array, texture_sampler, in.uv, in.texture_id);
    let final_color = base_color.rgb ;
    return vec4<f32>(final_color, 1.0);
} 