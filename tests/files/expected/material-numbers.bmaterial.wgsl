fn surface(s: SurfaceIn) -> Surface {
    let n10_value = 1.5e-7f;
    let n7_value = -0.0f;
    let n8_value = 7.0f;
    let n9_result = (n7_value + n8_value);
    let n2_vector = vec3<f32>(1e-5f, 0.0001f, 1e16f);
    let n3_vector = vec3<f32>(123456790.0f, 1e-45f, (-1e30f));
    let n5_result = (n2_vector + n3_vector);
    let n4_vector = vec3<f32>(3.4e38f, 0.1f, 0.33333334f);
    let n6_result = (n5_result + n4_vector);
    let n11_vector = vec3<f32>(1e16f, (-1e-5f), 65504.5f);
    let n12_result = (n6_result - n11_vector);
    return Surface(vec3<f32>(0.12345679f, 1e-10f, 2.5e17f), n10_value, n9_result, n12_result, 1.0f, s.normal);
}
const USES_LIGHTS: bool = false;
const CUSTOM_LIGHTING: bool = false;
const OUTLINE: vec4<f32> = vec4<f32>(vec3<f32>(0.0f, 0.0f, 0.0f), 0.0f);
