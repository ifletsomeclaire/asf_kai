use meshopt::{build_meshlets, VertexDataAdapter};

const MAX_VERTICES: usize = 64;
const MAX_TRIANGLES: usize = 128;

#[test]
fn test_build_meshlets_for_quad() {
    let vertices: &[f32] = &[
        // Quad vertices
        0.5, 0.5, 0.0, // top right
        0.5, -0.5, 0.0, // bottom right
        -0.5, -0.5, 0.0, // bottom left
        -0.5, 0.5, 0.0, // top left
    ];

    let indices: &[u32] = &[
        0, 1, 3, // first triangle
        1, 2, 3, // second triangle
    ];

    let vertex_data = unsafe {
        std::slice::from_raw_parts(
            vertices.as_ptr() as *const u8,
            vertices.len() * std::mem::size_of::<f32>(),
        )
    };

    let adapter = VertexDataAdapter::new(vertex_data, std::mem::size_of::<f32>() * 3, 0).unwrap();

    let meshlets = build_meshlets(indices, &adapter, MAX_VERTICES, MAX_TRIANGLES, 0.0);

    assert_eq!(meshlets.meshlets.len(), 1, "Expected to find one meshlet for a simple quad");

    let meshlet = &meshlets.meshlets[0];

    // The first meshlet should contain all vertices and indices for the quad.
    assert_eq!(meshlet.vertex_count, 4);
    assert_eq!(meshlet.triangle_count, 2);

    // Verify the indices within the meshlet. These are local to the meshlet's vertices.
    // The `vertices` array in the meshlet contains indices into the *original* vertex buffer.
    // So the indices in `triangles` should be 0, 1, 2, 3, but mapped from the original indices.
    let expected_meshlet_vertices: &[u32] = &[1, 2, 3, 0];
    let meshlet_vertices = &meshlets.vertices[meshlet.vertex_offset as usize..(meshlet.vertex_offset + meshlet.vertex_count) as usize];
    assert_eq!(meshlet_vertices, expected_meshlet_vertices);

    // The triangles are groups of 3 indices into the `meshlet.vertices` array.
    let expected_meshlet_triangles: &[u8] = &[
        0, 1, 2, // Corresponds to original indices 1, 2, 3
        3, 0, 2, // Corresponds to original indices 0, 1, 3
    ];
    let meshlet_triangles = &meshlets.triangles[meshlet.triangle_offset as usize..(meshlet.triangle_offset + meshlet.triangle_count * 3) as usize];
    assert_eq!(meshlet_triangles, expected_meshlet_triangles);
} 