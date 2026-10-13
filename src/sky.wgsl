// Independently authored port of the shipped sky.hlsl (SkyVS/SkyPS).
struct Globals {
    vp: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;
struct SkyDraw {
    world: mat4x4<f32>,
    color: vec4<f32>,
    color2: vec4<f32>,
};
@group(1) @binding(0) var<uniform> d: SkyDraw;
@group(1) @binding(1) var sky_texture: texture_2d<f32>;
@group(1) @binding(2) var sky_sampler: sampler;
struct Input {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
};
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};
@vertex fn sky_vertex(input: Input) -> Output {
    var o: Output;
    let wpos = d.world * vec4<f32>(input.position, 1.0);
    o.position = g.vp * wpos;
    // Snap to the far plane, as SkyVS does outside GLSLES.
    o.position.z = o.position.w - 1.0 / 16.0;
    o.uv = input.uv;
    o.color = input.color * mix(d.color2, d.color, wpos.y / 1700.0);
    return o;
}
@fragment fn sky_fragment(input: Output) -> @location(0) vec4<f32> {
    return textureSample(sky_texture, sky_sampler, input.uv) * input.color;
}
