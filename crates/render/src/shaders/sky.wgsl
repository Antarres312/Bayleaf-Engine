// The sky behind the scene: one triangle over the whole screen, at the far plane. Each pixel's
// view direction picks a colour between the horizon and the top (above) or the ground (below),
// with a soft glow around the first directional light.

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: VertexOut;
    out.clip = vec4<f32>(corners[index], 1.0, 1.0);
    out.ndc = corners[index];
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let near = globals.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let far = globals.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - near.xyz / near.w);
    var color: vec3<f32>;
    if (dir.y >= 0.0) {
        color = mix(globals.sky_horizon.rgb, globals.sky_zenith.rgb, pow(dir.y, 0.6));
    } else {
        color = mix(globals.sky_horizon.rgb, globals.sky_ground.rgb, pow(-dir.y, 0.4));
    }
    // The sun: the first light, when it is directional, as a small bright disc in a soft glow.
    if (u32(globals.ambient.w) > 0u && globals.lights[0].direction.w == 0.0) {
        let to_sun = -normalize(globals.lights[0].direction.xyz);
        let facing = max(dot(dir, to_sun), 0.0);
        let sun = globals.lights[0].radiance.rgb;
        color += sun * (pow(facing, 400.0) * 0.6 + pow(facing, 12.0) * 0.06);
    }
    return vec4<f32>(color, 1.0);
}
