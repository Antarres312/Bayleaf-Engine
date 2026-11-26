fn surface(s: SurfaceIn) -> Surface {
    let n3_result = (vec3<f32>(1.0f, 0.5f, 0.25f) + vec3<f32>(0.0f));
    return Surface(n3_result, 0.0f, 1.0f, vec3<f32>(0.0f, 0.0f, 0.0f), 0.5f, s.normal);
}
const USES_LIGHTS: bool = false;
const CUSTOM_LIGHTING: bool = false;
const OUTLINE: vec4<f32> = vec4<f32>(vec3<f32>(0.0f, 0.0f, 0.0f), 0.0f);
