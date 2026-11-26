fn surface(s: SurfaceIn) -> Surface {
    return Surface(vec3<f32>(0.8f, 0.8f, 0.8f), 0.0f, 0.5f, vec3<f32>(0.0f, 0.0f, 0.0f), 1.0f, s.normal);
}
const USES_LIGHTS: bool = false;
const CUSTOM_LIGHTING: bool = false;
const OUTLINE: vec4<f32> = vec4<f32>(vec3<f32>(0.0f, 0.0f, 0.0f), 0.0f);
