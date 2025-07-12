// GPU Picking Compute Shader
// This shader samples the ID texture at the specified coordinates and collects unique entity IDs

// Uniform buffer containing pick coordinates
@group(0) @binding(0) var<uniform> pick_coords: vec2<u32>;

// ID texture to sample from
@group(0) @binding(1) var id_texture: texture_2d<u32>;

// Buffer to track seen IDs (to avoid duplicates)
@group(0) @binding(2) var<storage, read_write> seen_ids: array<u32>;

// Buffer to store results (first element is count, rest are entity IDs)
@group(0) @binding(3) var<storage, read_write> results: array<u32>;

// Maximum number of results we can return
const MAX_RESULTS = 256u;
const MAX_SEEN_IDS = 1024u;

// Helper function to check if an ID has been seen
fn is_id_seen(id: u32) -> bool {
    for (var i = 0u; i < MAX_SEEN_IDS; i++) {
        if (seen_ids[i] == id) {
            return true;
        }
        // If we hit a zero, we've reached the end of the list
        if (seen_ids[i] == 0u) {
            break;
        }
    }
    return false;
}

// Helper function to add an ID to the seen list
fn add_seen_id(id: u32) {
    for (var i = 0u; i < MAX_SEEN_IDS; i++) {
        if (seen_ids[i] == 0u) {
            seen_ids[i] = id;
            break;
        }
    }
}

// Helper function to add a result
fn add_result(id: u32) {
    let current_count = results[0];
    if (current_count < MAX_RESULTS) {
        results[current_count + 1u] = id;
        results[0] = current_count + 1u;
    }
}

@compute @workgroup_size(1, 1, 1)
fn main() {
    // Initialize result count
    results[0] = 0u;
    
    // Sample the ID texture at the pick coordinates
    let id = textureLoad(id_texture, vec2<i32>(pick_coords));
    
    // If we got a valid ID (non-zero), add it to results
    if (id.x != 0u) {
        // Check if we've already seen this ID
        if (!is_id_seen(id.x)) {
            add_seen_id(id.x);
            add_result(id.x);
        }
    }
    
    // For more complex picking, you could sample a larger area around the point
    // and collect all unique IDs in that region
} 