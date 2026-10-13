struct Globals {
    screen: vec2<f32>,
    atlas: vec2<f32>,
    // Radius of the rounded bottom corners, 0 for square.
    corner: f32,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct Instance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) kind: u32,
};

struct Varyings {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) kind: u32,
};

// One quad per instance, drawn as a 4-vertex triangle strip.
@vertex
fn vs(@builtin(vertex_index) vi: u32, i: Instance) -> Varyings {
    let corner = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    let px = i.pos + corner * i.size;
    var v: Varyings;
    v.clip = vec4<f32>(px.x / g.screen.x * 2.0 - 1.0, 1.0 - px.y / g.screen.y * 2.0, 0.0, 1.0);
    v.uv = (i.uv.xy + corner * i.uv.zw) / g.atlas;
    v.color = i.color;
    v.kind = i.kind;
    return v;
}

const SOLID: u32 = 0u;
const MASK: u32 = 1u;

// How much of pixel `p` is inside the window's rounded bottom corners (antialiased).
fn coverage(p: vec2<f32>) -> f32 {
    let r = g.corner;
    let q = vec2<f32>(min(p.x, g.screen.x - p.x), g.screen.y - p.y);
    if r <= 0.0 || q.x >= r || q.y >= r {
        return 1.0;
    }
    return clamp(r - length(vec2<f32>(r) - q) + 0.5, 0.0, 1.0);
}

@fragment
fn fs(v: Varyings) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas, samp, v.uv);
    var out = texel; // color emoji
    if v.kind == SOLID {
        out = v.color;
    } else if v.kind == MASK {
        out = vec4<f32>(v.color.rgb, v.color.a * texel.a);
    }
    return vec4<f32>(out.rgb, out.a * coverage(v.clip.xy));
}
