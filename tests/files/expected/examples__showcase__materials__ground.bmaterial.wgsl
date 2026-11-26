fn surface(s: SurfaceIn) -> Surface {
    let n2_uv = vec3<f32>(s.uv, 0.0);
    let n3_result = (n2_uv * vec3<f32>(5.0f));
    let n4_sample = textureSample(t0, material_sampler, (n3_result).xy);
    let n4_color = n4_sample.rgb;
    let n4_alpha = n4_sample.a;
    let n5_result = mix(vec3<f32>(0.085f, 0.1f, 0.09f), n4_color, n4_alpha);
    return Surface(n5_result, 0.0f, 0.85f, vec3<f32>(0.0f, 0.0f, 0.0f), 1.0f, s.normal);
}
const USES_LIGHTS: bool = false;
const CUSTOM_LIGHTING: bool = false;
const OUTLINE: vec4<f32> = vec4<f32>(vec3<f32>(0.0f, 0.0f, 0.0f), 0.0f);
