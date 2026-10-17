//! RBX::Graphics::MaterialGenerator for Workspace Parts at the low-quality
//! level the renderer draws: which technique a Part's material and
//! Reflectance select, the material texture coordinates the geometry
//! generators write, and the textures the techniques sample.
//!
//! Recovered from:
//! - createDefaultMaterial 010c0e00: Plastic and SmoothPlastic
//!   (`material & !0x10 == 0x100`) with Reflectance > 0.015 take the
//!   reflection variant; every other Part takes the plain one;
//! - createRenderMaterial 010bc630: the low-quality technique (quality 2) is
//!   DefaultPlasticFS (Plastic, SmoothPlastic, or
//!   Default<Name>ReflectionFS with reflection), DefaultNeonFS (Neon),
//!   LowQMaterialWangFS (Cobblestone) or LowQMaterialFS (the rest). The
//!   LowQMaterial techniques bind rbxasset://textures/<lowercase
//!   name>/diffuse.dds (".dds" is the static string 0224dde0, initializer
//!   010c8530) to slot 5 with
//!   sampler (Linear, Wrap), Cobblestone adds wangIndex.dds (MaterialGenerator
//!   +0x2510) to slot 8 with (Point, Wrap), and the constant
//!   LqmatFarTilingFactor is (f, f, 1, 1) with f from the immediates at
//!   010bdcd0..010bdcf2;
//! - getTiling 010c3c40, read with `material::follow_switch`;
//! - the per-Part UV offset in addBlock 010cd104, addSphere 010cb4a4 and
//!   the Wedge generator 010dc8c6: bits 3..8 and 9..14 of the geometry seed
//!   pick two entries of the 64-float table 017f0218 (Brick and WoodPlanks use
//!   the bit values themselves);
//! - the glow factor (FadeDistance_GlowFactor.w): SceneManager::renderBegin
//!   (011c2200) uses the Glow settings' intensity, 2.2 from the SceneManager
//!   constructor immediate at 011c084e, while the PostFX level is below 1
//!   (1.0 otherwise). The low-quality renderer has no post effects.
//!
//! The native geometry seed is `part + (part >> 3)` for the PartInstance's
//! heap address (addPartImpl 010d1850). An address cannot be reproduced, so
//! `part_seed` stands in for it with a fixed hash of the Part's index.
use crate::{
    bundle::Bundle,
    dds::{Bc1Dds, Bc3Dds, luminance_alpha},
    geometry::MaterialUv,
    material::follow_switch,
    native_data::NativeImage,
    texture::PngTexture,
    workspace::Workspace,
};
use iced_x86::Register;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt::Write};

pub const PLASTIC: u32 = 0x100;
pub const SMOOTH_PLASTIC: u32 = 0x110;
pub const NEON: u32 = 0x120;
pub const WOOD_PLANKS: u32 = 0x210;
pub const BRICK: u32 = 0x350;
pub const COBBLESTONE: u32 = 0x370;

/// The 20 Part materials createRenderMaterial accepts (any other code returns
/// no material), with the name it builds shader and texture names from.
pub const RENDER_MATERIALS: [(u32, &str); 20] = [
    (0x100, "Plastic"),
    (0x110, "SmoothPlastic"),
    (0x120, "Neon"),
    (0x200, "Wood"),
    (0x210, "WoodPlanks"),
    (0x310, "Marble"),
    (0x320, "Slate"),
    (0x330, "Concrete"),
    (0x340, "Granite"),
    (0x350, "Brick"),
    (0x360, "Pebble"),
    (0x370, "Cobblestone"),
    (0x410, "Rust"),
    (0x420, "Diamondplate"),
    (0x430, "Aluminum"),
    (0x440, "Metal"),
    (0x500, "Grass"),
    (0x510, "Sand"),
    (0x520, "Fabric"),
    (0x600, "Ice"),
];

/// The low-quality technique a Part draws with.
#[derive(Clone, Debug, PartialEq)]
pub enum Technique {
    /// DefaultPlasticFS: the stud texture modulates the vertex color.
    /// `reflection` selects Default<Name>ReflectionFS (createRenderMaterial
    /// appends "Reflection" to the material name, so SmoothPlastic is
    /// DefaultSmoothPlasticReflectionFS), which also blends in the
    /// environment map by the vertex reflectance. Without reflection both
    /// materials use DefaultPlasticFS. `smooth` only changes the name: the
    /// low-quality branch of plastic.hlsl and smoothplastic.hlsl is the same.
    Plastic { reflection: bool, smooth: bool },
    /// DefaultNeonFS: the vertex color times the glow factor, unlit.
    Neon,
    /// LowQMaterialFS / LowQMaterialWangFS with the material's diffuse map.
    LowQuality {
        material: u32,
        folder: String,
        far_tiling: f32,
        wang: bool,
    },
}

impl Technique {
    pub fn name(&self) -> String {
        match self {
            Technique::Plastic {
                reflection: false, ..
            } => "DefaultPlasticFS".into(),
            Technique::Plastic {
                reflection: true,
                smooth: false,
            } => "DefaultPlasticReflectionFS".into(),
            Technique::Plastic {
                reflection: true,
                smooth: true,
            } => "DefaultSmoothPlasticReflectionFS".into(),
            Technique::Neon => "DefaultNeonFS".into(),
            Technique::LowQuality { wang: true, .. } => "LowQMaterialWangFS".into(),
            Technique::LowQuality { .. } => "LowQMaterialFS".into(),
        }
    }
}

fn render_name(material: u32) -> Result<&'static str, String> {
    RENDER_MATERIALS
        .iter()
        .find(|(code, _)| *code == material)
        .map(|(_, name)| *name)
        .ok_or_else(|| format!("material {material:#x} has no render material"))
}

/// LqmatFarTilingFactor.xy (createRenderMaterial 010bdc9a..010bdcf2):
/// 0.25 for Slate, Concrete, Granite, Aluminum, Grass and Ice, 0.5 for Rust
/// and 1 for the other LowQMaterial materials.
pub fn far_tiling(material: u32) -> f32 {
    match material {
        0x320 | 0x330 | 0x340 | 0x430 | 0x500 | 0x600 => 0.25,
        0x410 => 0.5,
        _ => 1.0,
    }
}

/// createDefaultMaterial (010c0e00) then createRenderMaterial's low-quality
/// technique.
pub fn technique(material: u32, reflectance: f32) -> Result<Technique, String> {
    let name = render_name(material)?;
    let reflection = material & !0x10 == PLASTIC && reflectance > 0.015;
    Ok(match material {
        PLASTIC | SMOOTH_PLASTIC => Technique::Plastic {
            reflection,
            smooth: material == SMOOTH_PLASTIC,
        },
        NEON => Technique::Neon,
        _ => Technique::LowQuality {
            material,
            folder: name.to_lowercase(),
            far_tiling: far_tiling(material),
            wang: material == COBBLESTONE,
        },
    })
}

/// A stand-in for the native geometry seed `part + (part >> 3)`: a 16-byte
/// aligned pseudo address hashed from the Part's index in the Workspace.
pub fn part_seed(index: usize) -> u32 {
    let mut z = (index as u32).wrapping_add(1).wrapping_mul(0x9e37_79b9);
    z = (z ^ (z >> 16)).wrapping_mul(0x85eb_ca6b);
    z = (z ^ (z >> 13)).wrapping_mul(0xc2b2_ae35);
    z ^= z >> 16;
    let address = (z & 0x0fff_fff0) | 0x0800_0000;
    address.wrapping_add(address >> 3)
}

pub struct MaterialTables {
    tiling: BTreeMap<u32, f32>,
    offsets: [f32; 64],
    /// SceneManager Glow intensity (011c084e).
    pub glow: f32,
}

impl MaterialTables {
    pub fn load(image: &NativeImage) -> Result<Self, String> {
        let mut tiling = BTreeMap::new();
        for (code, _) in RENDER_MATERIALS {
            let t = follow_switch(
                image,
                0x010c3c40,
                0x010c3e65,
                Register::EAX,
                None,
                Register::XMM0,
                code,
            )?;
            if !t.is_finite() || t <= 0.0 {
                return Err(format!("invalid native tiling for {code:#x}"));
            }
            tiling.insert(code, t);
        }
        let bytes = image.region(0x017f0218, 256)?;
        let offsets: [f32; 64] = std::array::from_fn(|i| {
            f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
        });
        if offsets.iter().any(|v| !(0.0..=1.0).contains(v)) {
            return Err("invalid native material UV offset table".into());
        }
        // MOV dword ptr [EAX + 0x4], imm32.
        let code = image.region(0x011c084e, 7)?;
        if code[..3] != [0xc7, 0x40, 0x04] {
            return Err("unexpected SceneManager glow initializer".into());
        }
        let glow = f32::from_le_bytes(code[3..7].try_into().unwrap());
        if !glow.is_finite() || glow <= 0.0 {
            return Err("invalid native glow intensity".into());
        }
        Ok(Self {
            tiling,
            offsets,
            glow,
        })
    }

    /// MaterialGenerator::getTiling (010c3c40).
    pub fn tiling(&self, material: u32) -> Result<f32, String> {
        self.tiling
            .get(&material)
            .copied()
            .ok_or_else(|| format!("material {material:#x} has no native tiling"))
    }

    /// The geometry generators' UV origin and scale for one Part. Brick and
    /// WoodPlanks start at the seed's bit values; the rest at
    /// (2 T[i] - 1) * (x + y + z) / 2, in the native float order.
    pub fn uv(&self, material: u32, size: [f32; 3], seed: u32) -> Result<MaterialUv, String> {
        let (a, b) = ((seed >> 3 & 0x3f) as usize, (seed >> 9 & 0x3f) as usize);
        let offset = if material == BRICK || material == WOOD_PLANKS {
            [a as f32, b as f32]
        } else {
            let s = ((size[0] + size[1]) + size[2]) * 0.5;
            let t = |i: usize| ((self.offsets[i] + self.offsets[i]) + -1.0) * s;
            [t(a), t(b)]
        };
        Ok(MaterialUv {
            tiling: self.tiling(material)?,
            offset,
        })
    }
}

/// The diffuse maps, Wang index and per-draw techniques of a Workspace.
pub struct MaterialScene {
    pub techniques: BTreeMap<String, Technique>,
    /// Vertex reflectance per draw: (int)(min(Reflectance, 1) * 0.8 * 255)
    /// / 255, 0 at or below 0 (addBlock 010cce01).
    pub reflectance: BTreeMap<String, f32>,
    pub textures: BTreeMap<u32, PngTexture>,
    pub wang: Option<PngTexture>,
    pub glow: f32,
    pub report: String,
}

impl MaterialScene {
    /// A draw's technique: the Workspace's own, or plain Plastic for the
    /// character and fixtures.
    pub fn technique(&self, name: &str, material: u32) -> Result<Technique, String> {
        match self.techniques.get(name) {
            Some(t) => Ok(t.clone()),
            None => technique(material, 0.0),
        }
    }
}

/// The vertex reflectance byte addBlock packs into Extra.a.
pub fn vertex_reflectance(reflectance: f32) -> f32 {
    let clamped = if reflectance <= 1.0 { reflectance } else { 1.0 };
    if 0.0 < reflectance {
        ((clamped * 0.8 * 255.0) as i32) as f32 / 255.0
    } else {
        0.0
    }
}

fn load_diffuse(bundle: &Bundle, folder: &str) -> Result<(PngTexture, String), String> {
    let path = format!("Contents/Resources/PlatformContent/pc/textures/{folder}/diffuse.dds");
    let bytes = bundle.read(&path, 16 * 1024 * 1024)?;
    let texture = match bytes.get(84..88) {
        Some(b"DXT1") => Bc1Dds::parse(&bytes)?.decode(),
        Some(b"DXT5") => Bc3Dds::parse(&bytes)?.decode(),
        _ => return Err(format!("{path}: unsupported DDS format")),
    };
    let first = &texture.mips[0];
    let line = format!(
        "material_texture folder={folder} format={} size={}x{} mips={} dds_sha256={:x}",
        String::from_utf8_lossy(&bytes[84..88]),
        first.width,
        first.height,
        texture.mips.len(),
        Sha256::digest(&bytes)
    );
    Ok((texture, line))
}

/// Techniques and textures for every Workspace Part.
pub fn workspace_materials(
    bundle: &Bundle,
    workspace: &Workspace,
) -> Result<MaterialScene, String> {
    let image = NativeImage::load(bundle)?;
    let tables = MaterialTables::load(&image)?;
    let mut scene = MaterialScene {
        techniques: BTreeMap::new(),
        reflectance: BTreeMap::new(),
        textures: BTreeMap::new(),
        wang: None,
        glow: tables.glow,
        report: String::new(),
    };
    for part in &workspace.parts {
        let t = technique(part.material, part.reflectance)
            .map_err(|e| format!("Part {:?}: {e}", part.name))?;
        if let Technique::LowQuality {
            material,
            folder,
            wang,
            ..
        } = &t
        {
            if !scene.textures.contains_key(material) {
                let (texture, line) = load_diffuse(bundle, folder)?;
                writeln!(scene.report, "{line}").unwrap();
                scene.textures.insert(*material, texture);
            }
            if *wang && scene.wang.is_none() {
                let bytes = bundle.read(
                    "Contents/Resources/PlatformContent/pc/textures/wangIndex.dds",
                    1024 * 1024,
                )?;
                let index = luminance_alpha(&bytes)?;
                writeln!(
                    scene.report,
                    "material_wang_index size={}x{} dds_sha256={:x}",
                    index.mips[0].width,
                    index.mips[0].height,
                    Sha256::digest(&bytes)
                )
                .unwrap();
                scene.wang = Some(index);
            }
        }
        writeln!(
            scene.report,
            "material_part name={:?} material={} technique={} reflectance={}",
            part.name,
            render_name(part.material)?,
            t.name(),
            vertex_reflectance(part.reflectance)
        )
        .unwrap();
        scene
            .reflectance
            .insert(part.name.clone(), vertex_reflectance(part.reflectance));
        scene.techniques.insert(part.name.clone(), t);
    }
    writeln!(
        scene.report,
        "workspace_material_pass parts={} textures={} wang={} glow={}",
        workspace.parts.len(),
        scene.textures.len(),
        scene.wang.is_some(),
        scene.glow
    )
    .unwrap();
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn techniques_follow_create_default_material() {
        assert_eq!(
            technique(PLASTIC, 0.015).unwrap(),
            Technique::Plastic {
                reflection: false,
                smooth: false
            }
        );
        assert_eq!(
            technique(SMOOTH_PLASTIC, 0.5).unwrap().name(),
            "DefaultSmoothPlasticReflectionFS"
        );
        assert_eq!(
            technique(PLASTIC, 0.5).unwrap().name(),
            "DefaultPlasticReflectionFS"
        );
        assert_eq!(
            technique(SMOOTH_PLASTIC, 0.0).unwrap().name(),
            "DefaultPlasticFS"
        );
        // Neon and the textured materials never reflect at low quality.
        assert_eq!(technique(NEON, 1.0).unwrap(), Technique::Neon);
        assert_eq!(
            technique(COBBLESTONE, 1.0).unwrap(),
            Technique::LowQuality {
                material: COBBLESTONE,
                folder: "cobblestone".into(),
                far_tiling: 1.0,
                wang: true
            }
        );
        assert_eq!(
            technique(0x410, 0.0).unwrap().name(),
            "LowQMaterialFS".to_string()
        );
        // Terrain-only materials have no render material.
        assert!(technique(0x530, 0.0).is_err());
    }
    #[test]
    fn vertex_reflectance_quantizes_like_add_block() {
        assert_eq!(vertex_reflectance(0.0), 0.0);
        assert_eq!(vertex_reflectance(-1.0), 0.0);
        assert_eq!(vertex_reflectance(1.0), 204.0 / 255.0);
        assert_eq!(vertex_reflectance(5.0), 204.0 / 255.0);
        assert_eq!(vertex_reflectance(0.5), 102.0 / 255.0);
    }
    #[test]
    fn seeds_are_stable_and_spread() {
        assert_eq!(part_seed(3), part_seed(3));
        let bits: std::collections::BTreeSet<u32> =
            (0..64).map(|i| part_seed(i) >> 3 & 0x3f).collect();
        assert!(bits.len() > 30);
    }
}
