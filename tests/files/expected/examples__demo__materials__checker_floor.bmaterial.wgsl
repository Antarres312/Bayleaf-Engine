fn surface(s: SurfaceIn) -> Surface {
    let n2_value = bl_checker((s.position + globals.origin.xyz) * 1.0f);
    let n3_result = mix(vec3<f32>(0.07f, 0.085f, 0.08f), vec3<f32>(0.13f, 0.16f, 0.14f), n2_value);
    return Surface(n3_result, 0.0f, 0.85f, vec3<f32>(0.0f, 0.0f, 0.0f), 1.0f, s.normal);
}
const USES_LIGHTS: bool = false;
const CUSTOM_LIGHTING: bool = false;
const OUTLINE: vec4<f32> = vec4<f32>(vec3<f32>(0.0f, 0.0f, 0.0f), 0.0f);
