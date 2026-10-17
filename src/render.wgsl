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
// High-quality constants. fade: FadeDistance_GlowFactor (shading distance,
// its inverse, the unused outline scale, glow). The c vectors hold the
// material's CFG_* values (see Gpu::add_materials), flags the CFG_OPT_*
// switches and the Reflection variant.
struct Hq {
    fade: vec4<f32>,
    c0: vec4<f32>,
    c1: vec4<f32>,
    c2: vec4<f32>,
    c3: vec4<f32>,
    flags: vec4<f32>,
};
@group(1) @binding(7) var<uniform> hq: Hq;
// Technique slots 5..8: DiffuseMap, NormalMap, SpecularMap and
// NormalDetailMap (or the Wang index), each with its own sampler state.
@group(1) @binding(8) var diffuse_map: texture_2d<f32>;
@group(1) @binding(9) var diffuse_sampler: sampler;
@group(1) @binding(10) var normal_map: texture_2d<f32>;
@group(1) @binding(11) var normal_sampler: sampler;
@group(1) @binding(12) var specular_map: texture_2d<f32>;
@group(1) @binding(13) var specular_sampler: sampler;
@group(1) @binding(14) var detail_map: texture_2d<f32>;
@group(1) @binding(15) var detail_sampler: sampler;
struct Input {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) specular: vec2<f32>,
    @location(5) studs: vec2<f32>,
    @location(6) reflectance: f32,
    @location(7) tangent: vec3<f32>,
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
    @location(10) tangent: vec3<f32>,
    // DefaultVS with PIN_HQ: lt.y * specular intensity, and the vertex
    // specular power (Normal_SpecPower.w).
    @location(11) specular_hq: vec2<f32>,
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
    o.tangent = input.tangent;
    o.specular_hq = vec2<f32>(select(0.0, input.specular.x, n_dot_l > 0.0), input.specular.y);
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

// ---- High-quality techniques (Technique lod 1) ----
// common.h nmapUnpack (non-GLSLES): xy from alpha and green.
fn nmap_unpack(value: vec4<f32>) -> vec3<f32> {
    let xy = value.ag * 2.0 - 1.0;
    return vec3<f32>(xy, sqrt(clamp(1.0 + dot(-xy, xy), 0.0, 1.0)));
}
// The (float)(half) cast of the specular term.
fn to_half(v: f32) -> f32 {
    return unpack2x16float(pack2x16float(vec2<f32>(v, 0.0))).x;
}
// DefaultPS from the albedo to the fog with PIN_HQ: per-pixel specular
// with power `power`, the reflection lerp, and fog. The outline multiply is
// 1 while outlines are off (OutlineBrightness_ShadowInfo.xy = (0, 1)).
fn shade_hq(input: Output, albedo_in: vec4<f32>, normal: vec3<f32>, diffuse_intensity: vec3<f32>,
            specular_intensity: f32, power: f32, reflectance: f32) -> vec4<f32> {
    let shadow = g.grid_shadow.w;
    let reflection = textureSample(environment, environment_sampler, reflect(-input.view, normal)).rgb;
    let albedo = mix(albedo_in.rgb, reflection, reflectance);
    let diffuse = (g.ambient.xyz + diffuse_intensity * shadow + g.grid_shadow.xyz) * albedo;
    let h = normalize(-g.lamp_dir.xyz + normalize(input.view));
    let highlight = to_half(pow(clamp(dot(normal, h), 0.0, 1.0), power));
    let specular = g.lamp0.xyz * (specular_intensity * shadow * highlight);
    let color = diffuse + specular;
    return vec4<f32>(mix(g.fog_color.xyz, color, clamp(input.fog, 0.0, 1.0)), albedo_in.a);
}
// PIN_SURFACE: the surface normal through the interpolated tangent frame,
// per-pixel diffuse.
fn shade_surface(input: Output, albedo: vec3<f32>, surface_normal: vec3<f32>, specular: f32,
                 gloss: f32, reflectance: f32) -> vec4<f32> {
    let bitangent = cross(input.normal, input.tangent);
    let normal = normalize(surface_normal.x * input.tangent + surface_normal.y * bitangent
        + surface_normal.z * input.normal);
    let n_dot_l = dot(normal, -g.lamp_dir.xyz);
    let diffuse = clamp(n_dot_l, 0.0, 1.0) * g.lamp0.xyz + max(-n_dot_l, 0.0) * g.lamp1.xyz;
    let specular_intensity = step(0.0, n_dot_l) * specular;
    return shade_hq(input, vec4<f32>(albedo, input.color.a), normal, diffuse, specular_intensity,
        gloss, reflectance);
}
// surfaceShaderExec's fade2: x over LQMAT_FADE_FACTOR (1/300), y over the
// shading distance.
fn fade2(input: Output) -> vec2<f32> {
    return vec2<f32>(clamp(1.0 - input.depth * (1.0 / 300.0), 0.0, 1.0),
        clamp(1.0 - input.depth * hq.fade.y, 0.0, 1.0));
}
// DefaultSmoothPlastic(Reflection)HQFS: PIN_PLASTIC with PIN_HQ.
@fragment fn fragment_plastic_hq(input: Output) -> @location(0) vec4<f32> {
    let atlas = textureSample(decal_texture, decal_sampler, input.studs);
    let albedo = vec4<f32>(input.color.xyz * (atlas.r * 2.0), input.color.a);
    let reflectance = select(0.0, input.reflectance, hq.flags.w > 0.5);
    return shade_hq(input, albedo, normalize(input.normal), input.diffuse, input.specular_hq.x,
        input.specular_hq.y, reflectance);
}
// DefaultPlastic(Reflection)HQFS: plastic.hlsl's surfaceShader. c0 =
// (CFG_TEXTURE_TILING, CFG_BUMP_INTENSITY, CFG_SPECULAR, CFG_GLOSS).
@fragment fn fragment_plastic_surface_hq(input: Output) -> @location(0) vec4<f32> {
    let fade = fade2(input).y;
    let studs = textureSample(diffuse_map, diffuse_sampler, input.studs);
    var normal = nmap_unpack(textureSample(normal_map, normal_sampler, input.studs));
    let noise = nmap_unpack(textureSample(detail_map, detail_sampler, input.uv * hq.c0.x));
    var noise_scale = max(input.color.a * 2.0 * hq.c0.y - 1.0 * hq.c0.y, 0.0);
    var reflectance = 0.0;
    if hq.flags.w > 0.5 {
        noise_scale *= clamp(1.0 - 2.0 * input.reflectance, 0.0, 1.0);
        reflectance = input.reflectance;
    }
    normal = vec3<f32>(normal.xy + noise.xy * noise_scale, normal.z);
    normal = vec3<f32>(normal.xy * fade, normal.z);
    return shade_surface(input, input.color.rgb * (studs.r * 2.0), normal, hq.c0.z, hq.c0.w,
        reflectance);
}
// material.hlsl sampleFar: the far-tiled sample faded in below the cutoff.
fn sample_far(t: texture_2d<f32>, s: sampler, uv: vec2<f32>, fade: f32, cutoff: f32) -> vec4<f32> {
    let near = textureSample(t, s, uv);
    let far = textureSample(t, s, uv * hq.c2.z);
    let cscale = 1.0 / (1.0 - cutoff);
    let blended = mix(far, near, clamp(fade * cscale - cutoff * cscale, 0.0, 1.0));
    return select(blended, near, cutoff == 0.0);
}
// material.hlsl's surfaceShader after the texture reads.
// c0 = (TEXTURE_TILING, DIFFUSE_SCALE, SPECULAR_SCALE, GLOSS_SCALE),
// c1 = (REFLECTION_SCALE, NORMAL_SHADOW_SCALE, SPECULAR_LOD, GLOSS_LOD),
// c2 = (NORMAL_DETAIL_TILING, NORMAL_DETAIL_SCALE, FAR_TILING,
// FAR_DIFFUSE_CUTOFF), c3 = (FAR_NORMAL_CUTOFF, FAR_SPECULAR_CUTOFF),
// flags = (OPT_DIFFUSE_CONST, OPT_NORMAL_CONST, OPT_BLEND_COLOR, -).
fn material_surface(input: Output, fade: f32, diffuse_in: vec4<f32>, normal_in: vec3<f32>,
                    specular_in: vec2<f32>) -> vec4<f32> {
    var diffuse = diffuse_in * hq.c0.y;
    if hq.flags.x > 0.5 {
        diffuse = vec4<f32>(1.0);
    }
    var normal = normal_in;
    if hq.flags.y > 0.5 {
        normal = vec3<f32>(0.0, 0.0, 1.0);
    }
    normal = vec3<f32>(normal.xy * fade, normal.z);
    let shadow_factor = 1.0 + normal.x * hq.c1.y;
    var albedo = input.color.rgb * diffuse.rgb * shadow_factor;
    if hq.flags.z > 0.5 {
        albedo = mix(vec3<f32>(1.0), input.color.rgb, diffuse.a) * diffuse.rgb * shadow_factor;
    }
    let studs = textureSample(decal_texture, decal_sampler, input.studs);
    albedo *= studs.r * 2.0;
    let specbase = specular_in * vec2<f32>(hq.c0.z, hq.c0.w) + vec2<f32>(0.0, 0.01);
    let specfade = mix(vec2<f32>(hq.c1.z, hq.c1.w), specbase, fade);
    return shade_surface(input, albedo, normal, specfade.x, specfade.y,
        specular_in.y * fade * hq.c1.x);
}
// Default<Material>HQFS (material.hlsl without CFG_WANG_TILES).
@fragment fn fragment_surface_hq(input: Output) -> @location(0) vec4<f32> {
    let f = fade2(input);
    let uv = input.uv * hq.c0.x;
    let diffuse = sample_far(diffuse_map, diffuse_sampler, uv, f.x, hq.c2.w);
    var normal = nmap_unpack(sample_far(normal_map, normal_sampler, uv, f.y, hq.c3.x));
    let detail = nmap_unpack(textureSample(detail_map, detail_sampler, uv * hq.c2.x));
    normal = vec3<f32>(normal.xy + detail.xy * hq.c2.y, normal.z);
    let specular = sample_far(specular_map, specular_sampler, uv, f.y, hq.c3.y).rg;
    return material_surface(input, f.y, diffuse, normal, specular);
}
// DefaultCobblestoneHQFS (CFG_WANG_TILES): getWang over the Wang index in
// slot 8 (.ba, 128 texels, tiling CFG_TEXTURE_TILING) and sampleWang.
@fragment fn fragment_surface_wang_hq(input: Output) -> @location(0) vec4<f32> {
    let f = fade2(input);
    let base = input.uv * hq.c0.x * 4.0;
    let wang = textureSample(detail_map, detail_sampler, base / 128.0).ba * (255.0 / 256.0);
    let uv = wang + fract(base) * 0.25;
    let dx = dpdx(base * 0.25);
    let dy = dpdy(base * 0.25);
    let diffuse = textureSampleGrad(diffuse_map, diffuse_sampler, uv, dx, dy);
    let normal = nmap_unpack(textureSampleGrad(normal_map, normal_sampler, uv, dx, dy));
    let specular = textureSampleGrad(specular_map, specular_sampler, uv, dx, dy).rg;
    return material_surface(input, f.y, diffuse, normal, specular);
}
