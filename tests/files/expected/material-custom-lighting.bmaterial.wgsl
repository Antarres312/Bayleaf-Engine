fn surface(s: SurfaceIn) -> Surface {
    let n2_direction = s.light_dir;
    let n3_normal = s.normal;
    let n4_result = dot(n2_direction, n3_normal);
    let n10_value = 1.0f;
    let n7_result = clamp(floor(n4_result * max(floor(n10_value), 2.0)) / (max(floor(n10_value), 2.0) - 1.0), 0.0, 1.0);
    let n2_color = s.light_color;
    let n8_color = s.other_light;
    let n9_result = (n2_color + n8_color);
    let n5_result = (vec3<f32>(n7_result) * n9_result);
    let n6_value = 0.5f;
    return Surface(vec3<f32>(0.0), 0.0, 1.0, n5_result, n6_value, s.normal);
}
const USES_LIGHTS: bool = true;
const CUSTOM_LIGHTING: bool = true;
const OUTLINE: vec4<f32> = vec4<f32>(vec3<f32>(0.1f, 0.1f, 0.2f), 0.05f);
