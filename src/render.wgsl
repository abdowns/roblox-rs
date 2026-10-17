// Independently authored low-quality static shading adapter. Light-grid/shadow
// terms are explicit uniform probe inputs; this is not the full 2016 pipeline.
struct Globals {
    vp: mat4x4<f32>,
    camera: vec4<f32>,
    ambient: vec4<f32>,
    lamp0: vec4<f32>,
    lamp1: vec4<f32>,
    lamp_dir: vec4<f32>,
    fog_color: vec4<f32>,
    fog: vec4<f32>,
    grid_shadow: vec4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var decal_texture: texture_2d<f32>;
@group(1) @binding(1) var decal_sampler: sampler;
// Low-quality material constants: LqmatFarTilingFactor and the glow factor
// (FadeDistance_GlowFactor.w) in glow.x.
struct Material {
    far_tiling: vec4<f32>,
    glow: vec4<f32>,
};
@group(1) @binding(2) var<uniform> m: Material;
@group(1) @binding(3) var wang_index: texture_2d<f32>;
@group(1) @binding(4) var wang_sampler: sampler;
@group(1) @binding(5) var environment: texture_cube<f32>;
@group(1) @binding(6) var environment_sampler: sampler;
struct Input {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) specular: vec2<f32>,
    @location(5) studs: vec2<f32>,
    @location(6) reflectance: f32,
};
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) diffuse: vec3<f32>,
    @location(2) specular: f32,
    @location(3) fog: f32,
    @location(4) uv: vec2<f32>,
    @location(5) studs: vec2<f32>,
    // View_Depth: camera - position and the clip w.
    @location(6) view: vec3<f32>,
    @location(7) depth: f32,
    @location(8) normal: vec3<f32>,
    @location(9) reflectance: f32,
};
@vertex fn vertex(input: Input) -> Output {
    var o: Output;
    o.position = g.vp * vec4<f32>(input.position, 1.0);
    o.color = input.color;
    o.uv = input.uv;
    o.studs = input.studs;
    let n_dot_l = dot(input.normal, -g.lamp_dir.xyz);
    let h = normalize(-g.lamp_dir.xyz + normalize(g.camera.xyz - input.position));
    var highlight = 0.0;
    if n_dot_l > 0.0 {
        highlight = pow(max(dot(h, input.normal), 0.0), input.specular.y);
    }
    o.diffuse = clamp(n_dot_l, 0.0, 1.0) * g.lamp0.xyz + max(-n_dot_l, 0.0) * g.lamp1.xyz;
    o.specular = highlight * input.specular.x;
    o.fog = (g.fog.z - o.position.w) * g.fog.w;
    o.view = g.camera.xyz - input.position;
    o.depth = o.position.w;
    o.normal = input.normal;
    o.reflectance = input.reflectance;
    return o;
}
@fragment fn fragment_plastic(input: Output) -> @location(0) vec4<f32> {
    let atlas = textureSample(decal_texture, decal_sampler, input.studs);
    let modulation = select(1.0, atlas.r * 2.0, input.studs.x >= 0.0);
    let albedo = input.color.xyz * modulation;
    let lighting = g.ambient.xyz + input.diffuse * g.grid_shadow.w + g.grid_shadow.xyz;
    let lit_color = lighting * albedo + g.lamp0.xyz * (input.specular * g.grid_shadow.w);
    return vec4<f32>(mix(g.fog_color.xyz, lit_color, clamp(input.fog, 0.0, 1.0)), input.color.a);
}
@fragment fn fragment(input: Output) -> @location(0) vec4<f32> {
    let lighting = g.ambient.xyz + input.diffuse * g.grid_shadow.w + g.grid_shadow.xyz;
    let lit_color = lighting * input.color.xyz + g.lamp0.xyz * (input.specular * g.grid_shadow.w);
    return vec4<f32>(mix(g.fog_color.xyz, lit_color, clamp(input.fog, 0.0, 1.0)), input.color.a);
}
@fragment fn fragment_decal(input: Output) -> @location(0) vec4<f32> {
    let albedo = textureSample(decal_texture, decal_sampler, input.uv) * input.color;
    let lighting = g.ambient.xyz + input.diffuse * g.grid_shadow.w + g.grid_shadow.xyz;
    let lit_color = lighting * albedo.xyz + g.lamp0.xyz * (input.specular * g.grid_shadow.w);
    return vec4<f32>(mix(g.fog_color.xyz, lit_color, clamp(input.fog, 0.0, 1.0)), albedo.a);
}
// default.hlsl DefaultPS with PIN_PLASTIC PIN_REFLECTION (not HQ): the stud
// albedo blended toward the environment map by the vertex reflectance.
@fragment fn fragment_plastic_reflection(input: Output) -> @location(0) vec4<f32> {
    let atlas = textureSample(decal_texture, decal_sampler, input.studs);
    let modulation = select(1.0, atlas.r * 2.0, input.studs.x >= 0.0);
    let reflection = textureSample(environment, environment_sampler, reflect(-input.view, input.normal)).rgb;
    let albedo = mix(input.color.xyz * modulation, reflection, input.reflectance);
    let lighting = g.ambient.xyz + input.diffuse * g.grid_shadow.w + g.grid_shadow.xyz;
    let lit_color = lighting * albedo + g.lamp0.xyz * (input.specular * g.grid_shadow.w);
    return vec4<f32>(mix(g.fog_color.xyz, lit_color, clamp(input.fog, 0.0, 1.0)), input.color.a);
}
// PIN_LOWQMAT: the diffuse map's alpha blends the vertex color in.
fn low_quality(input: Output, sampled: vec4<f32>) -> vec4<f32> {
    let albedo = mix(vec3<f32>(1.0), input.color.xyz, sampled.a) * sampled.xyz;
    let lighting = g.ambient.xyz + input.diffuse * g.grid_shadow.w + g.grid_shadow.xyz;
    let lit_color = lighting * albedo + g.lamp0.xyz * (input.specular * g.grid_shadow.w);
    return vec4<f32>(mix(g.fog_color.xyz, lit_color, clamp(input.fog, 0.0, 1.0)), input.color.a);
}
// LowQMaterialFS: sampleFar1 with CFG_FAR_DIFFUSE_CUTOFF 0.6 and
// LQMAT_FADE_FACTOR 1/300.
@fragment fn fragment_low_quality(input: Output) -> @location(0) vec4<f32> {
    let fade = clamp(1.0 - input.depth * (1.0 / 300.0), 0.0, 1.0);
    let cutoff = 0.6;
    let cscale = 1.0 / (1.0 - cutoff);
    let far = textureSample(decal_texture, decal_sampler, input.uv * m.far_tiling.xy);
    let near = textureSample(decal_texture, decal_sampler, input.uv);
    return low_quality(input, mix(far, near, clamp(fade * cscale - cutoff * cscale, 0.0, 1.0)));
}
// LowQMaterialWangFS: common.h getWang (tiling 1, a 128-texel index read
// as .ba) and sampleWang (explicit gradients).
@fragment fn fragment_low_quality_wang(input: Output) -> @location(0) vec4<f32> {
    let base = input.uv * 1.0 * 4.0;
    let wang = textureSample(wang_index, wang_sampler, base / 128.0).ba * (255.0 / 256.0);
    let uv = wang + fract(base) * 0.25;
    let sampled = textureSampleGrad(decal_texture, decal_sampler, uv, dpdx(base * 0.25), dpdy(base * 0.25));
    return low_quality(input, sampled);
}
// DefaultNeonFS (PIN_PLASTIC PIN_NEON): color times the glow factor.
@fragment fn fragment_neon(input: Output) -> @location(0) vec4<f32> {
    let fog = clamp(input.fog, 0.0, 1.0);
    let color = input.color.xyz * m.glow.x;
    return vec4<f32>(mix(g.fog_color.xyz, color, fog), 1.0 - fog * input.color.a);
}
