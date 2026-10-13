//! RBX::Graphics::Sky default path: skybox (Sky::render 010b6210), sun and
//! moon billboards and their colors (Sky::update 010b5560). Stars are only
//! drawn when the moon is up and are not implemented here.
use crate::{bundle::Bundle, dds::Bc1Dds, lighting::SkyParameters, texture::PngTexture};
use image::{ImageFormat, ImageReader, Limits};
use std::io::Cursor;

/// Sky ctor 010b3890 quad: position, white vertex color, uv (triangle strip).
pub const QUAD: [([f32; 3], [f32; 2]); 4] = [
    ([-1.0, -1.0, 0.0], [0.0, 1.0]),
    ([-1.0, 1.0, 0.0], [0.0, 0.0]),
    ([1.0, -1.0, 0.0], [1.0, 1.0]),
    ([1.0, 1.0, 0.0], [1.0, 0.0]),
];
const DISTANCE: f32 = 2700.0;
const SUN_SCALE: f32 = 500.0;
const MOON_SCALE: f32 = 250.0;
/// Sky instance defaults (009e7f18) in loadSkyBox slot order = NormalId 0..5.
const FACES: [&str; 6] = ["rt", "up", "bk", "lf", "dn", "ft"];

pub struct SkyTextures {
    pub faces: Vec<PngTexture>,
    pub sun: PngTexture,
    pub moon: PngTexture,
}

fn jpeg(bytes: &[u8]) -> Result<PngTexture, String> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg);
    let mut limits = Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    PngTexture::from_rgba(image.width(), image.height(), image.into_raw())
}

impl SkyTextures {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let faces = FACES
            .iter()
            .map(|face| {
                let bytes = bundle.read(
                    &format!(
                        "Contents/Resources/PlatformContent/pc/textures/sky/sky512_{face}.tex"
                    ),
                    4 * 1024 * 1024,
                )?;
                Ok(Bc1Dds::parse(&bytes)?.decode())
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            faces,
            sun: jpeg(&bundle.read("Contents/Resources/content/sky/sun.jpg", 1024 * 1024)?)?,
            moon: jpeg(&bundle.read("Contents/Resources/content/sky/moon.jpg", 1024 * 1024)?)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Blend {
    /// BlendState(One, Zero) for skybox faces.
    Opaque,
    /// BlendState(One, One) set before the sun and kept for the moon.
    Additive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkyDraw {
    /// 0..6 skybox faces, 6 sun, 7 moon.
    pub texture: usize,
    /// Column-major world matrix.
    pub world: [f32; 16],
    pub color: [f32; 4],
    pub color2: [f32; 4],
    pub blend: Blend,
}

fn world(rotation: [[f32; 3]; 3], scale: f32, position: [f32; 3]) -> [f32; 16] {
    let mut m = [0.0; 16];
    for c in 0..3 {
        for r in 0..3 {
            m[c * 4 + r] = rotation[r][c] * scale;
        }
    }
    m[12..15].copy_from_slice(&position);
    m[15] = 1.0;
    m
}
fn mul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}
/// FUN_006b3ea1: columns (up' x dir, up', dir) with up' = up orthogonalized.
fn billboard(dir: [f32; 3]) -> [[f32; 3]; 3] {
    // worldUp . dir, in the native component order.
    let d = 0.0 * dir[2] + 1.0 * dir[1] + 0.0 * dir[0];
    let mut up = [0.0 - dir[0] * d, 1.0 - dir[1] * d, 0.0 - d * dir[2]];
    let length = (up[2] * up[2] + up[1] * up[1] + up[0] * up[0]).sqrt();
    if length.abs() <= 0.001 {
        let z = dir[2];
        let v = [0.0 - dir[0] * z, 0.0 - dir[1] * z, 1.0 - z * dir[2]];
        let inv = 1.0 / (v[2] * v[2] + v[1] * v[1] + v[0] * v[0]).sqrt();
        up = v.map(|x| x * inv);
    } else {
        up = up.map(|x| x * (1.0 / length));
    }
    let x = [
        up[1] * dir[2] - up[2] * dir[1],
        up[2] * dir[0] - dir[2] * up[0],
        dir[1] * up[0] - up[1] * dir[0],
    ];
    std::array::from_fn(|r| [x[r], up[r], dir[r]])
}

/// Sky::update + Sky::render for the default (no Sky instance) case.
pub fn draws(sky: &SkyParameters, camera: [f32; 3]) -> Result<(Vec<SkyDraw>, u32), String> {
    let color = |c: [f32; 3]| [c[0], c[1], c[2], 1.0];
    let mut result = Vec::new();
    for face in 0..6u32 {
        let angle = match face {
            1 => std::f32::consts::FRAC_PI_2,
            4 => -std::f32::consts::FRAC_PI_2,
            _ => 0.0,
        };
        let (s, c) = angle.sin_cos();
        let ry = [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]];
        let mirror = [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let normal = crate::rig::normal_frame(face)?.rotation;
        let m = mul(mul(ry, mirror), normal);
        let position = std::array::from_fn(|r| m[r][2] * DISTANCE + camera[r]);
        result.push(SkyDraw {
            texture: face as usize,
            world: world(m, DISTANCE, position),
            color: color(sky.sky_ambient),
            color2: color(sky.sky_horizon),
            blend: Blend::Opaque,
        });
    }
    // Sky::update: sun color fades in as the sun rises past -0.1.
    let fade = ((sky.true_sun[1] + 0.1) * 10.0).clamp(0.0, 1.0);
    let sun_color = sky.emissive.map(|v| v * 0.8 * fade);
    let moon_level = ((1.0 - sky.sky_ambient[1]) + 0.1).min(1.0);
    let place = |dir: [f32; 3]| -> ([f32; 3], [[f32; 3]; 3]) {
        let position: [f32; 3] = std::array::from_fn(|r| dir[r] * DISTANCE + camera[r]);
        let v: [f32; 3] = std::array::from_fn(|r| position[r] - camera[r]);
        let inv = 1.0 / (v[2] * v[2] + v[1] * v[1] + v[0] * v[0]).sqrt();
        (position, billboard(v.map(|x| x * inv)))
    };
    let (sun_position, sun_rotation) = place(sky.true_sun);
    // The sun is drawn three times with additive blending.
    for _ in 0..3 {
        result.push(SkyDraw {
            texture: 6,
            world: world(sun_rotation, SUN_SCALE, sun_position),
            color: color(sun_color),
            color2: color(sun_color),
            blend: Blend::Additive,
        });
    }
    // The moon only sets Color; Color2 keeps the sun color.
    let (moon_position, moon_rotation) = place(sky.true_moon);
    result.push(SkyDraw {
        texture: 7,
        world: world(moon_rotation, MOON_SCALE, moon_position),
        color: [moon_level, moon_level, moon_level, 1.0],
        color2: color(sun_color),
        blend: Blend::Additive,
    });
    // Star field level from the simple moon height (0 visible .. 3 none).
    let moon_y = sky.moon[1];
    let stars = if moon_y <= -0.3 {
        3
    } else if moon_y <= -0.1 {
        2
    } else if moon_y <= 0.2 {
        1
    } else {
        0
    };
    Ok((result, stars))
}
