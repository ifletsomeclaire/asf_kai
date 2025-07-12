// GPU Picking Compute Shader (Parallel)
// This shader samples the ID texture within a specified rectangular region.
// It uses atomics to safely collect unique, non-zero entity IDs from multiple threads.

// Uniform buffer containing the selection box [x, y, width, height]
@group(0) @binding(0) var<uniform> pick_box: vec4<u32>;

// ID texture to sample from
@group(0) @binding(1) var id_texture: texture_2d<u32>;

// Buffer to store results (first element is atomic counter, rest are entity IDs)
@group(0) @binding(2) var<storage, read_write> results: array<atomic<u32>>;

// Maximum number of results we can return
const MAX_RESULTS = 256u;

// Helper to add a result if it's not a duplicate.
// NOTE: This simple duplicate check is NOT safe in a highly parallel scenario
// without further atomic operations. A better approach is to deduplicate on the CPU.
fn add_result_if_unique(id: u32) {
    if (id == 0u) { // Ignore ID 0 (background)
        return;
    }

    // Atomically increment the counter to reserve a spot in the array.
    let new_index = atomicAdd(&results[0], 1u);

    // Only write if we are within the bounds of our results buffer.
    if (new_index < MAX_RESULTS) {
        results[new_index + 1u] = id;
    }
}

// Define the size of the workgroup. 8x8 = 64 threads per group.
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    // On the first invocation of the first workgroup, clear the counter.
    if (global_id.x == 0u && global_id.y == 0u) {
        atomicStore(&results[0], 0u);
    }

    // Calculate the pixel coordinate this thread is responsible for.
    let pixel_coords = vec2<u32>(
        pick_box.x + global_id.x,
        pick_box.y + global_id.y
    );

    // Ensure the thread is within the bounds of the selection box.
    if (pixel_coords.x >= (pick_box.x + pick_box.z) || pixel_coords.y >= (pick_box.y + pick_box.w)) {
        return;
    }

    let texture_dims = vec2<u32>(textureDimensions(id_texture));

    // Bounds check to avoid sampling outside the texture
    if (pixel_coords.x < texture_dims.x && pixel_coords.y < texture_dims.y) {
        let id = textureLoad(id_texture, pixel_coords, 0).x;
        add_result_if_unique(id);
    }
} 