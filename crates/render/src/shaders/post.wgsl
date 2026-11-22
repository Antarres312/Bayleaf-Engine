// The finished picture: the scene's high-range colour, with the frame's effects, written for
// display. With every effect off it is the colour as drawn, clamped.
//
// Tone mapping is the filmic curve fitted by Narkowicz (2015) to the ACES reference transform;
// ambient occlusion compares the depth around each pixel, without normals; antialiasing finds
// edges by the brightness of the neighbours and blends across them.

struct Post {
    // x: exposure, y: tone mapping (1 on), z: bloom strength, w: bloom threshold
    a: vec4<f32>,
    // x: contrast, y: saturation, z: vignette strength, w: vignette smoothness
    b: vec4<f32>,
    // rgb: depth fog colour, w: 1 with depth fog
    fog: vec4<f32>,
    // x: fog start, y: fog end, z: occlusion strength, w: occlusion radius (world units)
    c: vec4<f32>,
    // x: 1 with antialiasing, yz: one pixel in uv, w: 1 with bloom
    d: vec4<f32>,
};

@group(1) @binding(0) var scene_color: texture_2d<f32>;
@group(1) @binding(1) var bloom: texture_2d<f32>;
@group(1) @binding(2) var scene_depth: texture_depth_multisampled_2d;
@group(1) @binding(3) var post_sampler: sampler;
@group(1) @binding(4) var<uniform> post: Post;

struct FullOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> FullOut {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: FullOut;
    out.clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn filmic(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Exposure, bloom, tone mapping and the colour adjustments: what a pixel of high-range colour shows as.
fn graded(hdr: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    var c = hdr;
    if (post.d.w > 0.5) {
        c = c + textureSampleLevel(bloom, post_sampler, uv, 0.0).rgb * post.a.z;
    }
    c = c * post.a.x;
    if (post.a.y > 0.5) {
        c = filmic(c);
    } else {
        c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    c = (c - vec3<f32>(0.5)) * post.b.x + vec3<f32>(0.5);
    c = mix(vec3<f32>(luma(c)), c, post.b.y);
    return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn depth_at(uv: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(scene_depth));
    let p = clamp(vec2<i32>(uv * size), vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
    return textureLoad(scene_depth, p, 0);
}

// The world position a depth stands for at `uv`.
fn world_at(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let w = globals.inv_view_proj * ndc;
    return w.xyz / w.w;
}

// How open the point at `uv` is: 1 open, less where nearer surfaces crowd round it.
fn openness(uv: vec2<f32>, depth: f32) -> f32 {
    let here = world_at(uv, depth);
    let distance_here = distance(here, globals.camera_pos.xyz);
    // The radius in uv: the world radius as it looks from this far away.
    let reach = clamp(post.c.w / max(distance_here, 0.01) * 0.5, 0.002, 0.1);
    var blocked = 0.0;
    for (var i = 0; i < 12; i = i + 1) {
        let angle = f32(i) * 2.39996;
        let r = reach * (f32(i) + 1.0) / 12.0;
        let offset = vec2<f32>(cos(angle), sin(angle)) * r * vec2<f32>(post.d.z / post.d.y, 1.0);
        let d = depth_at(uv + offset);
        if (d >= 1.0) {
            continue;
        }
        let there = world_at(uv + offset, d);
        let nearer = distance_here - distance(there, globals.camera_pos.xyz);
        // Closer than this point, and not far in front of it (a shape far in front casts no occlusion).
        if (nearer > 0.02 * post.c.w && nearer < post.c.w * 2.0) {
            blocked = blocked + 1.0;
        }
    }
    return 1.0 - post.c.z * blocked / 12.0;
}

@fragment
fn fs_post(in: FullOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    var hdr = textureSampleLevel(scene_color, post_sampler, uv, 0.0);
    let depth = depth_at(uv);
    if (depth < 1.0) {
        if (post.c.z > 0.0) {
            hdr = vec4<f32>(hdr.rgb * openness(uv, depth), hdr.a);
        }
        if (post.fog.w > 0.5) {
            let d = distance(world_at(uv, depth), globals.camera_pos.xyz);
            let t = clamp((d - post.c.x) / max(post.c.y - post.c.x, 1e-4), 0.0, 1.0);
            hdr = vec4<f32>(mix(hdr.rgb, post.fog.rgb, t), hdr.a);
        }
    }
    var c = graded(hdr.rgb, uv);
    if (post.d.x > 0.5) {
        // Edges by brightness: blend a pixel with its neighbours where they differ sharply.
        let px = post.d.yz;
        let n = graded(textureSampleLevel(scene_color, post_sampler, uv - vec2<f32>(0.0, px.y), 0.0).rgb, uv);
        let s = graded(textureSampleLevel(scene_color, post_sampler, uv + vec2<f32>(0.0, px.y), 0.0).rgb, uv);
        let e = graded(textureSampleLevel(scene_color, post_sampler, uv + vec2<f32>(px.x, 0.0), 0.0).rgb, uv);
        let w = graded(textureSampleLevel(scene_color, post_sampler, uv - vec2<f32>(px.x, 0.0), 0.0).rgb, uv);
        let lumas = vec4<f32>(luma(n), luma(s), luma(e), luma(w));
        let low = min(luma(c), min(min(lumas.x, lumas.y), min(lumas.z, lumas.w)));
        let high = max(luma(c), max(max(lumas.x, lumas.y), max(lumas.z, lumas.w)));
        let contrast = high - low;
        if (contrast > max(0.05, high * 0.125)) {
            let amount = clamp(contrast * 2.0, 0.0, 0.5);
            c = mix(c, (n + s + e + w) * 0.25, amount);
        }
    }
    if (post.b.z > 0.0) {
        let from_centre = length((uv - vec2<f32>(0.5)) * vec2<f32>(1.0, post.d.z / post.d.y));
        let fade = smoothstep(0.8, 0.8 - max(post.b.w, 0.01), from_centre * (1.0 + post.b.z));
        c = c * mix(1.0 - post.b.z, 1.0, fade);
    }
    return vec4<f32>(c, hdr.a);
}

// Bloom: the bright parts kept (above the threshold), then blurred down a chain of half-size
// pictures and back up.
@fragment
fn fs_bright(in: FullOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(scene_color, post_sampler, in.uv, 0.0).rgb;
    let over = max(luma(c) - post.a.w, 0.0) / max(luma(c), 1e-4);
    return vec4<f32>(c * over, 1.0);
}

// A 13-tap filter, wider than a pixel of the source, for each step down.
@fragment
fn fs_down(in: FullOut) -> @location(0) vec4<f32> {
    let px = 1.0 / vec2<f32>(textureDimensions(scene_color));
    var sum = textureSampleLevel(scene_color, post_sampler, in.uv, 0.0).rgb * 0.5;
    for (var i = 0; i < 4; i = i + 1) {
        let corner = vec2<f32>(select(-1.0, 1.0, (i & 1) == 1), select(-1.0, 1.0, (i & 2) == 2));
        sum = sum + textureSampleLevel(scene_color, post_sampler, in.uv + corner * px, 0.0).rgb * 0.125;
    }
    return vec4<f32>(sum, 1.0);
}

// A tent filter for each step up, added onto the larger picture.
@fragment
fn fs_up(in: FullOut) -> @location(0) vec4<f32> {
    let px = 1.0 / vec2<f32>(textureDimensions(scene_color));
    var sum = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let weight = (2.0 - abs(f32(x))) * (2.0 - abs(f32(y))) / 16.0;
            sum = sum + textureSampleLevel(scene_color, post_sampler, in.uv + vec2<f32>(f32(x), f32(y)) * px, 0.0).rgb * weight;
        }
    }
    return vec4<f32>(sum, 1.0);
}
