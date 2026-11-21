// Shadow map: the depth of every opaque shape as seen from the light that casts shadows.

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct InstanceIn {
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
};

@vertex
fn vs_main(v: VertexIn, i: InstanceIn) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(i.model_0, i.model_1, i.model_2, i.model_3);
    return globals.light_view_proj * model * vec4<f32>(v.position, 1.0);
}
