//! Recovered straight-alpha PNG/mipmap and explicit character decal paths.
use crate::{
    bundle::Bundle,
    geometry::{Draw, RenderScene},
};
use image::{ImageFormat, ImageReader, Limits};
use sha2::{Digest, Sha256};
use std::{fmt::Write, io::Cursor};

#[derive(Clone)]
pub struct Mip {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone)]
pub struct PngTexture {
    pub mips: Vec<Mip>,
}
impl PngTexture {
    /// Current recovered resize path is square power-of-two RGBA8 PNG only.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") || bytes.get(24..26) != Some(&[8, 6]) {
            return Err("texture probe requires an RGBA8 PNG".into());
        }
        let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
        let mut limits = Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        // Native libpng applies neither gamma correction nor premultiplication.
        let image = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
        Self::from_rgba(image.width(), image.height(), image.into_raw())
    }
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, String> {
        if width != height
            || !width.is_power_of_two()
            || width > 4096
            || rgba.len() != width as usize * height as usize * 4
        {
            return Err("mipmap probe requires a bounded square power-of-two RGBA image".into());
        }
        let mut mips = vec![Mip {
            width,
            height,
            rgba,
        }];
        while mips.last().unwrap().width > 1 {
            let source = mips.last().unwrap();
            let side = source.width / 2;
            let mut rgba = Vec::with_capacity(side as usize * side as usize * 4);
            for y in 0..side {
                for x in 0..side {
                    for c in 0..4 {
                        let pixel = |dx: u32, dy: u32| {
                            source.rgba
                                [(((y * 2 + dy) * source.width + x * 2 + dx) * 4 + c) as usize]
                                as u16
                        };
                        // FUN_01134190: pairwise rounded-up average, horizontally
                        // then vertically. Not a single rounded four-pixel average.
                        let top = (pixel(0, 0) + pixel(1, 0)).div_ceil(2);
                        let bottom = (pixel(0, 1) + pixel(1, 1)).div_ceil(2);
                        rgba.push((top + bottom).div_ceil(2) as u8);
                    }
                }
            }
            mips.push(Mip {
                width: side,
                height: side,
                rgba,
            });
        }
        Ok(Self { mips })
    }
}

#[derive(Clone, bevy::prelude::Component)]
pub struct TexturedDraw {
    pub draw: Draw,
    pub texture: PngTexture,
}
pub struct DecalScene {
    pub draws: Vec<TexturedDraw>,
    pub report: String,
}

/// File-mesh decals reuse the whole mesh and UVs, irrespective of Face.
pub fn character_decals(bundle: &Bundle, scene: &RenderScene) -> Result<DecalScene, String> {
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let doc = roxmltree::Document::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut result = DecalScene {
        draws: Vec::new(),
        report: String::new(),
    };
    for item in doc
        .descendants()
        .filter(|n| n.has_tag_name("Item") && n.attribute("class") == Some("Decal"))
    {
        let properties = item
            .children()
            .find(|n| n.has_tag_name("Properties"))
            .ok_or("missing decal properties")?;
        let property = |name| {
            properties
                .children()
                .find(|n| n.attribute("name") == Some(name))
                .ok_or_else(|| format!("missing decal {name}"))
        };
        let name = property("Name")?.text().unwrap_or("");
        let content = property("Texture")?;
        let url = content
            .children()
            .find(|n| n.has_tag_name("url"))
            .and_then(|n| n.text())
            .unwrap_or("");
        if url.is_empty() {
            writeln!(
                result.report,
                "decal_skip name={name:?} reason=empty_texture"
            )
            .unwrap();
            continue;
        }
        let parent = item
            .parent()
            .and_then(|n| n.attribute("referent"))
            .ok_or("missing decal parent")?;
        let instance = scene
            .scene
            .instances
            .iter()
            .find(|i| i.referent == parent)
            .ok_or("unknown decal parent")?;
        let mut draw = scene
            .draws
            .iter()
            .find(|d| d.name == instance.name)
            .ok_or("decal parent has no visible geometry")?
            .clone();
        if draw.geometry != "special_head" {
            return Err("block/other decal UV generation is not implemented".into());
        }
        let transparency: f32 = property("Transparency")?
            .text()
            .unwrap_or("")
            .trim()
            .parse::<f32>()
            .map_err(|e| e.to_string())?;
        if !transparency.is_finite() || !(0.0..=1.0).contains(&transparency) {
            return Err("invalid decal transparency".into());
        }
        let face: u32 = property("Face")?
            .text()
            .unwrap_or("")
            .trim()
            .parse::<u32>()
            .map_err(|e| e.to_string())?;
        if face > 5 {
            return Err("invalid decal Face".into());
        }
        let path = url
            .strip_prefix("rbxasset://")
            .ok_or("offline decal requires rbxasset URL")?;
        let texture = PngTexture::parse(&bundle.read(
            &format!("Contents/Resources/content/{path}"),
            16 * 1024 * 1024,
        )?)?;
        let alpha = (255 - (transparency * 255.0) as u32) as f32 / 255.0;
        for vertex in &mut draw.vertices {
            vertex.color = [1.0, 1.0, 1.0, alpha];
        }
        draw.name = format!("{}/{}", instance.name, name);
        let first = &texture.mips[0];
        writeln!(result.report,"decal_loaded name={:?} face={face} vertices={} triangles={} size={}x{} mips={} rgba_sha256={:x} straight_alpha=true",draw.name,draw.vertices.len(),draw.indices.len()/3,first.width,first.height,texture.mips.len(),Sha256::digest(&first.rgba)).unwrap();
        for (level, mip) in texture.mips.iter().enumerate() {
            writeln!(
                result.report,
                "decal_mip level={level} size={}x{} sha256={:x}",
                mip.width,
                mip.height,
                Sha256::digest(&mip.rgba)
            )
            .unwrap();
        }
        result.draws.push(TexturedDraw { draw, texture });
    }
    writeln!(
        result.report,
        "decal_import_pass draws={}",
        result.draws.len()
    )
    .unwrap();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_mips_use_two_ceil_averages_and_preserve_straight_alpha() {
        let texture = PngTexture::from_rgba(
            2,
            2,
            vec![0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 1, 1, 255, 1],
        )
        .unwrap();
        assert_eq!(texture.mips[1].rgba, [1, 1, 255, 1]);
        assert!(PngTexture::from_rgba(0, 0, vec![]).is_err());
        assert!(PngTexture::from_rgba(2, 1, vec![0; 8]).is_err());
    }
}
