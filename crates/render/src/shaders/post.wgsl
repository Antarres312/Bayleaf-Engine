// The finished picture: the scene's high-range colour, with the frame's effects, written for
// display. With every effect off it is the colour as drawn, clamped.
//
// Tone mapping is the filmic curve fitted by Narkowicz (2015) to the ACES reference transform;
// ambient occlusion samples the half sphere above each point (its surface's normal worked out
// from the depth around it) and counts the samples that the visible surface hides; antialiasing finds
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
    // x: antialiasing (0 off, 1 fast, 2 fine), yz: one pixel in uv, w: 1 with bloom
    d: vec4<f32>,
    // x: colour lookup strength, y: its steps per side (0 without one)
    e: vec4<f32>,
};

@group(1) @binding(0) var scene_color: texture_2d<f32>;
@group(1) @binding(1) var bloom: texture_2d<f32>;
@group(1) @binding(2) var scene_depth: texture_depth_multisampled_2d;
@group(1) @binding(3) var post_sampler: sampler;
@group(1) @binding(4) var<uniform> post: Post;
// N squares of N by N side by side: blue picks the square, red runs across it and green down it.
@group(1) @binding(5) var lookup: texture_2d<f32>;

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

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(high, low, c <= vec3<f32>(0.0031308));
}

fn from_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

// A colour (linear, 0 to 1) through the lookup image: the two squares either side of its blue,
// each sampled between pixels, then blended.
fn looked_up(c: vec3<f32>) -> vec3<f32> {
    let steps = post.e.y;
    let s = clamp(to_srgb(c), vec3<f32>(0.0), vec3<f32>(1.0)) * (steps - 1.0);
    let square = floor(s.b);
    let next = min(square + 1.0, steps - 1.0);
    let v = (s.g + 0.5) / steps;
    let width = steps * steps;
    let first = textureSampleLevel(lookup, post_sampler, vec2<f32>((square * steps + s.r + 0.5) / width, v), 0.0).rgb;
    let second = textureSampleLevel(lookup, post_sampler, vec2<f32>((next * steps + s.r + 0.5) / width, v), 0.0).rgb;
    return from_srgb(mix(first, second, s.b - square));
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
    c = clamp(mix(vec3<f32>(luma(c)), c, post.b.y), vec3<f32>(0.0), vec3<f32>(1.0));
    if (post.e.y > 1.5) {
        c = mix(c, looked_up(c), post.e.x);
    }
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

// The surface's normal at `uv` (a point `here`), from the points of the pixels beside it: on each
// axis the nearer neighbour, so an edge behind or in front does not bend it. It faces the eye.
fn normal_at(uv: vec2<f32>, here: vec3<f32>) -> vec3<f32> {
    let px = post.d.yz;
    let right = world_at(uv + vec2<f32>(px.x, 0.0), depth_at(uv + vec2<f32>(px.x, 0.0)));
    let left = world_at(uv - vec2<f32>(px.x, 0.0), depth_at(uv - vec2<f32>(px.x, 0.0)));
    let down = world_at(uv + vec2<f32>(0.0, px.y), depth_at(uv + vec2<f32>(0.0, px.y)));
    let up = world_at(uv - vec2<f32>(0.0, px.y), depth_at(uv - vec2<f32>(0.0, px.y)));
    let across = select(here - left, right - here, distance(right, here) < distance(here, left));
    let along = select(here - up, down - here, distance(down, here) < distance(here, up));
    var n = normalize(cross(along, across));
    if (dot(n, globals.camera_pos.xyz - here) < 0.0) {
        n = -n;
    }
    return n;
}

// Where world point `p` shows: its uv on the screen, and its distance from the eye.
fn on_screen(p: vec3<f32>) -> vec3<f32> {
    let clip = globals.view_proj * vec4<f32>(p, 1.0);
    let ndc = clip.xy / max(clip.w, 1e-6);
    return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, distance(p, globals.camera_pos.xyz));
}

// A number from 0 to 1 that changes from pixel to pixel, to turn each pixel's samples differently.
fn pixel_noise(uv: vec2<f32>) -> f32 {
    let p = floor(uv / post.d.yz);
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// How open the point at `uv` is: 1 open, less where surfaces nearby hide the half sphere above it
// (a corner where a wall meets the floor, a crease, the ground under a low shape).
fn openness(uv: vec2<f32>, depth: f32) -> f32 {
    let here = world_at(uv, depth);
    let n = normal_at(uv, here);
    let side = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9);
    let t = normalize(cross(side, n));
    let b = cross(n, t);
    let radius = post.c.w;
    let turn = pixel_noise(uv) * 6.2831853;
    // Lifted a little off the surface, so it does not hide itself.
    let start = here + n * radius * 0.03;
    let distance_here = distance(here, globals.camera_pos.xyz);
    var blocked = 0.0;
    for (var i = 0; i < 16; i = i + 1) {
        let k = (f32(i) + 0.5) / 16.0;
        // Spread over the half sphere: round it by the golden angle, from low to high.
        let angle = f32(i) * 2.39996 + turn;
        let height = 0.1 + 0.9 * fract(k * 7.0);
        let flat_part = sqrt(1.0 - height * height);
        let direction = t * (cos(angle) * flat_part) + b * (sin(angle) * flat_part) + n * height;
        // Nearer samples count more often: closer surfaces darken more.
        let reach = radius * mix(0.15, 1.0, k * k);
        let sample = start + direction * reach;
        let seen = on_screen(sample);
        if (any(seen.xy < vec2<f32>(0.0)) || any(seen.xy > vec2<f32>(1.0))) {
            continue;
        }
        let d = depth_at(seen.xy);
        if (d >= 1.0) {
            continue;
        }
        let surface = distance(world_at(seen.xy, d), globals.camera_pos.xyz);
        // The visible surface there stands in front of the sample, and not far in front of this
        // point (a shape far nearer the eye casts no occlusion here).
        if (surface < seen.z - radius * 0.02) {
            blocked = blocked + smoothstep(0.0, 1.0, radius / max(abs(distance_here - surface), 1e-4));
        }
    }
    return 1.0 - post.c.z * clamp(blocked / 16.0 * 1.5, 0.0, 1.0);
}

// The finished colour of the scene at `uv` (between pixels, filtered).
fn graded_at(uv: vec2<f32>) -> vec3<f32> {
    return graded(textureSampleLevel(scene_color, post_sampler, uv, 0.0).rgb, uv);
}

// Fine antialiasing: where the brightness of the diagonal neighbours differs enough to be an edge,
// the colour is averaged along the edge's own direction (worked out from those neighbours), a
// little and further; the further average is kept unless it strays past the neighbours' range.
fn fine_edges(uv: vec2<f32>, here: vec3<f32>) -> vec3<f32> {
    let px = post.d.yz;
    let nw = luma(graded_at(uv + vec2<f32>(-px.x, -px.y)));
    let ne = luma(graded_at(uv + vec2<f32>(px.x, -px.y)));
    let sw = luma(graded_at(uv + vec2<f32>(-px.x, px.y)));
    let se = luma(graded_at(uv + vec2<f32>(px.x, px.y)));
    let m = luma(here);
    let low = min(m, min(min(nw, ne), min(sw, se)));
    let high = max(m, max(max(nw, ne), max(sw, se)));
    if (high - low < max(0.0312, high * 0.125)) {
        return here;
    }
    var along = vec2<f32>(-((nw + ne) - (sw + se)), (nw + sw) - (ne + se));
    let reduce = max((nw + ne + sw + se) * 0.03125, 1.0 / 128.0);
    let scale = 1.0 / (min(abs(along.x), abs(along.y)) + reduce);
    along = clamp(along * scale, vec2<f32>(-8.0), vec2<f32>(8.0)) * px;
    let near = 0.5 * (graded_at(uv + along * (1.0 / 3.0 - 0.5)) + graded_at(uv + along * (2.0 / 3.0 - 0.5)));
    let far = near * 0.5 + 0.25 * (graded_at(uv - along * 0.5) + graded_at(uv + along * 0.5));
    let far_luma = luma(far);
    if (far_luma < low || far_luma > high) {
        return near;
    }
    return far;
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
    if (post.d.x > 1.5) {
        c = fine_edges(uv, c);
    } else if (post.d.x > 0.5) {
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
