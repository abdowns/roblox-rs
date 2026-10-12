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
struct Input {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) specular: vec2<f32>,
    @location(5) studs: vec2<f32>,
};
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) diffuse: vec3<f32>,
    @location(2) specular: f32,
    @location(3) fog: f32,
    @location(4) uv: vec2<f32>,
    @location(5) studs: vec2<f32>,
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
    return o;
}
@fragment fn fragment_plastic(input: Output) -> @location(0) vec4<f32> {
    let atlas = textureSample(decal_texture, decal_sampler, input.studs);
    let albedo = input.color.xyz * (atlas.r * 2.0);
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
