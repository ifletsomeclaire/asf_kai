// GPU Data Layout
// ------------------------------------------------------------------

struct VertexInput {
    @location(0) position: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec4<f32>,
};

// Describes a unique mesh's location in the global buffers.
struct MeshDescription {
    index_count: u32,
    first_index: u32,
    base_vertex: i32,
    _padding: u32,
};

// Describes a single object to be rendered.
struct Instance {
    model_matrix: mat4x4<f32>,
    mesh_id: u32,
};

// The pre-calculated lookup table for the shader.
struct InstanceLookup {
    instance_id: u32,
    first_vertex_of_instance: u32,
    _padding: vec2<u32>,
};

struct ViewProjection {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> u_view_proj: ViewProjection;

// Bindings for mesh and instance data
@group(1) @binding(0) var<storage, read> global_vertices: array<VertexInput>;
@group(1) @binding(1) var<storage, read> global_indices: array<u32>;
@group(1) @binding(2) var<storage, read> mesh_descriptions: array<MeshDescription>;
@group(1) @binding(3) var<storage, read> instances: array<Instance>;
@group(1) @binding(4) var<storage, read> instance_lookups: array<InstanceLookup>;

// Shader Entry Point
// ------------------------------------------------------------------

@vertex
fn vs_main(@builtin(vertex_index) global_vertex_id: u32) -> VertexOutput {
    // 1. Find which instance this vertex belongs to.
    let lookup = instance_lookups[global_vertex_id];
    let instance_id = lookup.instance_id;

    // 2. Fetch the data for that specific instance.
    let instance_data = instances[instance_id];
    let mesh_id = instance_data.mesh_id;
    let model_matrix = instance_data.model_matrix;

    // 3. Fetch the description for the required mesh.
    let mesh = mesh_descriptions[mesh_id];

    // 4. Calculate which vertex within the mesh this is (e.g., the 5th, 100th, etc.).
    let local_vertex_id = global_vertex_id - lookup.first_vertex_of_instance;

    // 5. Find the location of this vertex's index in the global index buffer.
    let index_location = mesh.first_index + local_vertex_id;

    // 6. Pull the final vertex index from the global index buffer.
    let final_vertex_index = global_indices[index_location];

    // 7. Pull the vertex data from the global vertex buffer.
    let vertex_data = global_vertices[u32(mesh.base_vertex) + final_vertex_index];

    // 8. Apply the instance's transform and project.
    var out: VertexOutput;
    let world_pos = model_matrix * vertex_data.position;
    out.world_pos = world_pos;
    out.clip_position = u_view_proj.view_proj * world_pos;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = in.world_pos.xyz * 0.5 + 0.5;
    return vec4<f32>(color, 1.0);
} 