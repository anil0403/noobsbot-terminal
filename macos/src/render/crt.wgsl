// CRT post-process: the terminal frame, drawn once more through a picture tube.

struct Params {
    screen: vec2<f32>,
    time: f32,
    scanlines: f32,
    glow: f32,
    curvature: f32,
    vignette: f32,
    flicker: f32,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var frame: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct Varyings {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle covering the screen.
@vertex
fn vs(@builtin(vertex_index) i: u32) -> Varyings {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var v: Varyings;
    v.clip = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    v.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return v;
}

const PI: f32 = 3.14159265;

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(frame, samp, uv, 0.0).rgb;
}

@fragment
fn fs(v: Varyings) -> @location(0) vec4<f32> {
    // Barrel distortion, scaled so the middle of each edge stays put: all the
    // text stays visible and the corners curve away into black.
    // Same formula as `warp` in crt.rs, which maps the mouse pointer.
    var c = v.uv * 2.0 - 1.0;
    c = c * (1.0 + p.curvature * dot(c, c)) / (1.0 + p.curvature);
    let uv = c * 0.5 + 0.5;

    var color = tap(uv);

    // Glow: light bleeding from bright text, two rings of samples.
    let px = 1.0 / p.screen;
    var blur = vec3<f32>(0.0);
    for (var i = 0; i < 8; i++) {
        let dir = vec2<f32>(cos(f32(i) * PI / 4.0), sin(f32(i) * PI / 4.0));
        blur += tap(uv + dir * px * 2.0) + tap(uv + dir * px * 5.0);
    }
    blur /= 16.0;
    color += blur * blur * p.glow; // squared: bright text bleeds, dark background doesn't

    // Scanlines every 3 pixel rows, following the curved screen.
    let line = 0.5 + 0.5 * cos(uv.y * p.screen.y * 2.0 * PI / 3.0);
    color *= 1.0 - p.scanlines * line;

    // Darker corners, and a faint 60 Hz shimmer.
    color *= 1.0 - p.vignette * dot(c, c) * 0.5;
    color *= 1.0 - p.flicker * (0.5 + 0.5 * sin(p.time * 2.0 * PI * 60.0));

    // Beyond the tube's edge: black.
    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
    return vec4<f32>(select(vec3<f32>(0.0), color, inside), 1.0);
}
