//-- Per-Vertex Data -----------------------------------------------------------
struct SkinnedVertex {
    position: vec4<f32>,
    normal: vec4<f32>,
    uv: vec2<f32>,
    _padding: vec2<f32>, // Padding to match Rust struct
    bone_indices: vec4<u32>,
    bone_weights: vec4<f32>,
};

//-- Static Asset Data ---------------------------------------------------------
struct MeshletDescription {
    vertex_list_offset: u32,
    triangle_list_offset: u32,
    triangle_count: u32,
    vertex_count: u32,
};

//-- Per-Frame/Per-Draw Data ----------------------------------------------------
struct AnimatedDrawCommand {
    meshlet_id: u32,
    bone_set_id: u32, // An index pointing to the start of a block of 256 matrices
    transform_id: u32,
    texture_id: u32,
};

//-- Vertex to Fragment Data ---------------------------------------------------
struct VSOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) texture_id: u32,
};

//-- Bindings ------------------------------------------------------------------

// @group(0): Per-View Data
@group(0) @binding(0) var<uniform> camera: mat4x4<f32>;

// @group(1): Static Mesh Data (provided by AssetServer)
@group(1) @binding(0) var<storage, read> vertices: array<SkinnedVertex>;
@group(1) @binding(1) var<storage, read> meshlet_vertex_indices: array<u32>;
@group(1) @binding(2) var<storage, read> meshlet_triangle_indices: array<u32>; // u8s packed into u32s
@group(1) @binding(3) var<storage, read> meshlet_descriptions: array<MeshletDescription>;

// @group(2): Per-Draw Data
@group(2) @binding(0) var<storage, read> indirection_buffer: array<AnimatedDrawCommand>;
@group(2) @binding(1) var<storage, read> bone_matrices: array<mat4x4<f32>>; // Bone matrices now include world transform

// @group(3): Texture Data (provided by AssetServer)
@group(3) @binding(0) var texture_array: texture_2d_array<f32>;
@group(3) @binding(1) var texture_sampler: sampler;

//-- Vertex Shader -------------------------------------------------------------

@vertex
fn vs_main(
    @builtin(instance_index) instance_id: u32, // The index of the meshlet draw command we're executing.
    @builtin(vertex_index) local_vtx_id: u32   // The index of the vertex within this meshlet's triangles.
) -> VSOutput {
    var output: VSOutput;
    output.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0); // Default to outside clip space

    // 1. Fetch the draw command for this specific meshlet draw.
    let command = indirection_buffer[instance_id];

    // 2. Use the command to get the meshlet's metadata.
    let meshlet = meshlet_descriptions[command.meshlet_id];

    // 3. Cull padded vertices.
    if (local_vtx_id >= meshlet.triangle_count * 3u) {
        return output;
    }

    // 4. Perform the two-level index lookup to find the final vertex.
    let total_byte_offset = meshlet.triangle_list_offset + local_vtx_id;
    let u32_index = total_byte_offset / 4u;
    let byte_in_u32 = total_byte_offset % 4u;
    let packed_indices = meshlet_triangle_indices[u32_index];
    let local_vertex_index_in_meshlet = (packed_indices >> (byte_in_u32 * 8u)) & 0xFFu;

    let vertex_index_in_list = meshlet.vertex_list_offset + local_vertex_index_in_meshlet;
    let final_vertex_index = meshlet_vertex_indices[vertex_index_in_list];

    let vertex = vertices[final_vertex_index];

    // 5. Calculate the skinning transform.
    var skin_transform: mat4x4<f32>;
    
    // Calculate total weight for normalization
    var total_weight = 0.0;
    for (var i = 0; i < 4; i = i + 1) {
        total_weight += vertex.bone_weights[i];
    }
    
    if (total_weight > 0.0) {
        skin_transform = mat4x4<f32>(
            0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0,
        );
        // Use the `bone_set_id` from the command as a direct offset into the bone matrices array.
        let bone_offset = command.bone_set_id;
        
        // Apply bone transformations with proper weight normalization
        for (var i = 0; i < 4; i = i + 1) {
            let bone_index = vertex.bone_indices[i];
            let bone_weight = vertex.bone_weights[i];
            if (bone_weight > 0.0 && bone_index < 256u) {
                // Normalize the weight and blend the bone matrices
                let normalized_weight = bone_weight / total_weight;
                skin_transform += bone_matrices[bone_offset + bone_index] * normalized_weight;
            }
        }
    } else {
        // If no weights, use identity matrix
        skin_transform = mat4x4<f32>(
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        );
    }

    // 6. Apply transformations.
    // The bone matrices now include the world transform, so we don't need to apply model_transform again
    let skinned_pos = skin_transform * vertex.position;
    let world_pos = skinned_pos; // Bone matrices already include world transform

    output.clip_position = camera * world_pos;
    
    // Safely calculate the world normal
    let skinned_normal = skin_transform * vec4<f32>(vertex.normal.xyz, 0.0);
    let world_normal_unnormalized = skinned_normal.xyz; // Bone matrices already include world transform

    // Prevent normalization of a zero vector
    if (length(world_normal_unnormalized) > 0.0001) {
        output.world_normal = normalize(world_normal_unnormalized);
    } else {
        // Provide a default normal if the calculated one is zero
        output.world_normal = vec3<f32>(0.0, 0.0, 1.0);
    }
    
    output.uv = vertex.uv;
    output.texture_id = command.texture_id;

    return output;
}

//-- Fragment Shader -----------------------------------------------------------

@fragment
fn fs_main(in: VSOutput) -> @location(0) vec4<f32> {
    let base_color = textureSample(texture_array, texture_sampler, in.uv, in.texture_id);
    // Basic lighting
    let light_dir = normalize(vec3<f32>(0.5, 1.0, 0.5));
    let diffuse_light = max(dot(in.world_normal, light_dir), 0.1) + 0.1; // Adding ambient term
    let final_color = base_color.rgb * diffuse_light;
    return vec4<f32>(final_color, 1.0);
}