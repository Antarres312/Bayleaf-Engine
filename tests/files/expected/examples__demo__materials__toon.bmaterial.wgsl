fn surface(s: SurfaceIn) -> Surface {
    let n9_color = s.color.rgb;
    let n3_normal = s.normal;
    let n2_direction = s.light_dir;
    let n4_result = dot(n3_normal, n2_direction);
    let n5_result = (0.0f + (n4_result - (-1.0f)) / select(1.0f - (-1.0f), 1e-6, abs(1.0f - (-1.0f)) < 1e-6) * (1.0f - 0.0f));
    let n7_result = clamp(floor(n5_result * max(floor(3.0f), 2.0)) / (max(floor(3.0f), 2.0) - 1.0), 0.0, 1.0);
    let n8_result = (0.35f + (n7_result - 0.0f) / select(1.0f - 0.0f, 1e-6, abs(1.0f - 0.0f) < 1e-6) * (1.0f - 0.35f));
    let n2_shadow = s.light_shadow;
    let n15_result = (0.6f + (n2_shadow - 0.0f) / select(1.0f - 0.0f, 1e-6, abs(1.0f - 0.0f) < 1e-6) * (1.0f - 0.6f));
    let n6_result = (n8_result * n15_result);
    let n10_result = (n9_color * vec3<f32>(n6_result));
    let n11_color = s.other_light;
    let n12_result = (n11_color * vec3<f32>(0.5f));
    let n13_result = (n12_result * n9_color);
    let n14_result = (n10_result + n13_result);
    return Surface(vec3<f32>(0.0), 0.0, 1.0, n14_result, 1.0f, s.normal);
}
const USES_LIGHTS: bool = true;
const CUSTOM_LIGHTING: bool = true;
const OUTLINE: vec4<f32> = vec4<f32>(vec3<f32>(0.04f, 0.03f, 0.06f), 0.04f);
