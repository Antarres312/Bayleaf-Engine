fn surface(s: SurfaceIn) -> Surface {
    let n2_color = s.color.rgb;
    let n3_factor = pow(1.0 - clamp(dot(s.normal, s.view), 0.0, 1.0), max(3.0f, 0.0));
    let n8_result = (vec3<f32>(n3_factor) * n2_color);
    let n4_seconds = s.time;
    let n5_result = (n4_seconds * 3.0f);
    let n6_result = sin(n5_result);
    let n7_result = mix(0.4f, 1.0f, n6_result);
    let n9_result = (n8_result * vec3<f32>(n7_result));
    return Surface(n2_color, 0.0f, 0.3f, n9_result, 1.0f, s.normal);
}
const USES_LIGHTS: bool = false;
const CUSTOM_LIGHTING: bool = false;
const OUTLINE: vec4<f32> = vec4<f32>(vec3<f32>(0.0f, 0.0f, 0.0f), 0.0f);
