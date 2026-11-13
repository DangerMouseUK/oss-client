// Batched sprite renderer: textured, tinted quads in physical pixel coordinates.

struct Globals {
    screen: vec2<f32>,
    camera: vec2<f32>,
    scale: f32,
    time: f32,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var out: VsOut;
    let ndc = v.pos / globals.screen * 2.0 - vec2<f32>(1.0, 1.0);
    out.clip = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    out.uv = v.uv;
    out.color = v.color;
    return out;
}

// Every sprite here is nearest-neighbour pixel art, so what the fragment wants
// is one texel, not a filtered sample. `textureSample` asks the rasteriser for
// a filtered read whatever the sampler's filter modes say, and on a software
// adapter that read costs about forty times a direct fetch: measured on WARP at
// 43 ns a textured pixel against 2.6 ns, which was three quarters of the frame.
// `fs_sampled` below is the same shader through the sampler, kept so the two can
// be compared on any machine (`OSS_GFX_SAMPLE=sample`); on a real GPU they cost
// the same and produce the same pixels.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(tex, 0));
    let texel = vec2<i32>(clamp(in.uv * size, vec2<f32>(0.0), size - 1.0));
    let c = textureLoad(tex, texel, 0) * in.color;
    if (c.a <= 0.0) {
        discard;
    }
    return c;
}

@fragment
fn fs_sampled(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv) * in.color;
    if (c.a <= 0.0) {
        discard;
    }
    return c;
}

// Starfield: two parallax layers of single-pixel stars, generated from a hash of the
// layer-space pixel so it is stable, infinite and costs no textures.

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn hash(x: i32, y: i32, layer: u32) -> u32 {
    var h = bitcast<u32>(x) * 0x8da6b343u ^ bitcast<u32>(y) * 0xd8163841u ^ layer * 0xcb1ab31fu;
    h = h ^ (h >> 16u);
    h = h * 0x7feb352du;
    h = h ^ (h >> 15u);
    h = h * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return h;
}

fn star(game_px: vec2<f32>, speed: f32, layer: u32) -> bool {
    let p = floor(game_px + floor(globals.camera * speed));
    // About 100 stars per 1024x1024 pixels per layer.
    return hash(i32(p.x), i32(p.y), layer) % 10486u == 0u;
}

@fragment
fn fs_stars(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let game_px = floor((frag.xy - globals.screen * 0.5) / globals.scale);
    if (star(game_px, 0.5, 0u)) {
        let v = 184.0 / 255.0;
        return vec4<f32>(v, v, v, 1.0);
    }
    if (star(game_px, 0.25, 1u)) {
        let v = 96.0 / 255.0;
        return vec4<f32>(v, v, v, 1.0);
    }
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}
