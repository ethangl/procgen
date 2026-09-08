// WGSL mirror of procgen-cubesphere's cross-face field sampling.
// Consumers provide cubesphere_load_field_texel for their storage layout.

struct CubesphereFieldTexel {
    channels_0: vec4<f32>,
    channels_1: vec4<f32>,
}

struct CubesphereFieldSample5 {
    channel_0: ScalarFieldSample3,
    channel_1: ScalarFieldSample3,
    channel_2: ScalarFieldSample3,
    channel_3: ScalarFieldSample3,
    channel_4: ScalarFieldSample3,
}

fn cubesphere_face_to_texel(coordinate: f32, resolution: u32) -> f32 {
    return (coordinate + 1.0) * 0.5 * f32(resolution) - 0.5;
}

fn cubesphere_texel_to_face(texel: f32, resolution: u32) -> f32 {
    return 2.0 * (texel + 0.5) / f32(resolution) - 1.0;
}

fn cubesphere_texel_index(face: u32, x: u32, y: u32, resolution: u32) -> u32 {
    return (face * resolution + y) * resolution + x;
}

fn cubesphere_nearest_texel(coordinates: CubesphereFaceCoordinates, resolution: u32) -> CubesphereFieldTexel {
    let x = u32(clamp(round(cubesphere_face_to_texel(coordinates.u, resolution)), 0.0, f32(resolution) - 1.0));
    let y = u32(clamp(round(cubesphere_face_to_texel(coordinates.v, resolution)), 0.0, f32(resolution) - 1.0));
    return cubesphere_load_field_texel(cubesphere_texel_index(coordinates.face, x, y, resolution));
}

fn cubesphere_add_texels(left: CubesphereFieldTexel, right: CubesphereFieldTexel) -> CubesphereFieldTexel {
    return CubesphereFieldTexel(left.channels_0 + right.channels_0, left.channels_1 + right.channels_1);
}

fn cubesphere_scale_texel(texel: CubesphereFieldTexel, scale: f32) -> CubesphereFieldTexel {
    return CubesphereFieldTexel(texel.channels_0 * scale, texel.channels_1 * scale);
}

fn cubesphere_field_tap(source_face: u32, x: i32, y: i32, resolution: u32) -> CubesphereFieldTexel {
    let edge = i32(resolution);
    let x_inside = x >= 0 && x < edge;
    let y_inside = y >= 0 && y < edge;
    if x_inside && y_inside {
        return cubesphere_load_field_texel(cubesphere_texel_index(source_face, u32(x), u32(y), resolution));
    }
    if !x_inside && !y_inside {
        let corner = cubesphere_unit_direction(CubesphereFaceCoordinates(
            source_face,
            select(1.0, -1.0, x < 0),
            select(1.0, -1.0, y < 0),
        ));
        var sum = CubesphereFieldTexel(vec4(0.0), vec4(0.0));
        var count = 0.0;
        for (var face = 0u; face < 6u; face++) {
            if dot(corner, cubesphere_face_frame(face).normal) > 0.0 {
                sum = cubesphere_add_texels(sum, cubesphere_nearest_texel(cubesphere_project_direction(corner, face), resolution));
                count += 1.0;
            }
        }
        return cubesphere_scale_texel(sum, 1.0 / count);
    }
    let outside = CubesphereFaceCoordinates(
        source_face,
        cubesphere_texel_to_face(f32(x), resolution),
        cubesphere_texel_to_face(f32(y), resolution),
    );
    let adjacent_direction = cubesphere_unit_direction(outside);
    let adjacent = cubesphere_project_direction(adjacent_direction, cubesphere_dominant_face(adjacent_direction));
    return cubesphere_nearest_texel(adjacent, resolution);
}

fn cubesphere_field_channel(texel: CubesphereFieldTexel, channel: u32) -> f32 {
    if channel < 4u { return texel.channels_0[channel]; }
    return texel.channels_1.x;
}

fn cubesphere_bilinear_channel(
    lower_left: CubesphereFieldTexel,
    lower_right: CubesphereFieldTexel,
    upper_left: CubesphereFieldTexel,
    upper_right: CubesphereFieldTexel,
    channel: u32,
    tx: f32,
    ty: f32,
    tx_derivative: vec3<f32>,
    ty_derivative: vec3<f32>,
) -> ScalarFieldSample3 {
    let ll = cubesphere_field_channel(lower_left, channel);
    let lr = cubesphere_field_channel(lower_right, channel);
    let ul = cubesphere_field_channel(upper_left, channel);
    let ur = cubesphere_field_channel(upper_right, channel);
    let lower = ll + (lr - ll) * tx;
    let upper = ul + (ur - ul) * tx;
    let along_x = (lr - ll) * (1.0 - ty) + (ur - ul) * ty;
    let along_y = (ul - ll) * (1.0 - tx) + (ur - lr) * tx;
    return ScalarFieldSample3(
        lower + (upper - lower) * ty,
        tx_derivative * along_x + ty_derivative * along_y,
    );
}

fn cubesphere_sample_field(direction: vec3<f32>, resolution: u32) -> CubesphereFieldSample5 {
    let face = cubesphere_dominant_face(direction);
    let coordinates = cubesphere_project_direction(direction, face);
    let x = cubesphere_face_to_texel(coordinates.u, resolution);
    let y = cubesphere_face_to_texel(coordinates.v, resolution);
    let x_floor = floor(x);
    let y_floor = floor(y);
    let tx = x - x_floor;
    let ty = y - y_floor;
    let x0 = i32(x_floor);
    let y0 = i32(y_floor);
    let lower_left = cubesphere_field_tap(face, x0, y0, resolution);
    let lower_right = cubesphere_field_tap(face, x0 + 1, y0, resolution);
    let upper_left = cubesphere_field_tap(face, x0, y0 + 1, resolution);
    let upper_right = cubesphere_field_tap(face, x0 + 1, y0 + 1, resolution);

    let frame = cubesphere_face_frame(face);
    let depth = dot(direction, frame.normal);
    let inverse_depth_squared = 1.0 / (depth * depth);
    let side_u = dot(direction, frame.u_axis);
    let side_v = dot(direction, frame.v_axis);
    let ratio_u = side_u / depth;
    let ratio_v = side_v / depth;
    let derivative_u = (frame.u_axis * depth - frame.normal * side_u)
        * (inverse_depth_squared / (CUBESPHERE_PI_OVER_FOUR * (1.0 + ratio_u * ratio_u)));
    let derivative_v = (frame.v_axis * depth - frame.normal * side_v)
        * (inverse_depth_squared / (CUBESPHERE_PI_OVER_FOUR * (1.0 + ratio_v * ratio_v)));
    let texel_scale = f32(resolution) * 0.5;
    let tx_derivative = derivative_u * texel_scale;
    let ty_derivative = derivative_v * texel_scale;
    return CubesphereFieldSample5(
        cubesphere_bilinear_channel(lower_left, lower_right, upper_left, upper_right, 0u, tx, ty, tx_derivative, ty_derivative),
        cubesphere_bilinear_channel(lower_left, lower_right, upper_left, upper_right, 1u, tx, ty, tx_derivative, ty_derivative),
        cubesphere_bilinear_channel(lower_left, lower_right, upper_left, upper_right, 2u, tx, ty, tx_derivative, ty_derivative),
        cubesphere_bilinear_channel(lower_left, lower_right, upper_left, upper_right, 3u, tx, ty, tx_derivative, ty_derivative),
        cubesphere_bilinear_channel(lower_left, lower_right, upper_left, upper_right, 4u, tx, ty, tx_derivative, ty_derivative),
    );
}
