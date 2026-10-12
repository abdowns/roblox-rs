//! Explicit character Plastic stud coordinates, recovered from addBlock.
use crate::{
    bundle::Bundle, dds::Bc1Dds, geometry::RenderScene, native_data::NativeImage,
    texture::PngTexture,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt::Write};

pub struct SurfaceScene {
    pub coordinates: BTreeMap<String, Vec<[f32; 2]>>,
    pub atlas: PngTexture,
    pub report: String,
}

fn face_uv(
    width: f32,
    height: f32,
    surface: u32,
    rows: &[f32; 6],
) -> Result<[[f32; 2]; 4], String> {
    if surface == 0 || surface == 10 {
        return Ok([[0.0; 2]; 4]);
    }
    if !(1..=5).contains(&surface) || height > 8.0 {
        return Err("unsupported stud surface or subdivision".into());
    }
    let u = width * 0.5;
    let v = rows[surface as usize];
    let extent = (height * 0.125).clamp(0.0, 1.0) * 0.25;
    Ok([[u, v], [0.0, v], [u, v + extent], [0.0, v + extent]])
}

pub fn character(bundle: &Bundle, scene: &RenderScene) -> Result<SurfaceScene, String> {
    let image = NativeImage::load(bundle)?;
    let rows = image.surface_rows()?;
    let bytes = bundle.read(
        "Contents/Resources/PlatformContent/pc/textures/studs.dds",
        16 * 1024 * 1024,
    )?;
    let dds = Bc1Dds::parse(&bytes)?;
    let mut result = SurfaceScene {
        coordinates: BTreeMap::new(),
        atlas: dds.decode(),
        report: format!(
            "stud_atlas_loaded width={} height={} mips={} decoder=native_squish_fallback dds_sha256={:x}\n",
            dds.width,
            dds.height,
            dds.mips.len(),
            Sha256::digest(&bytes)
        ),
    };
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let doc = roxmltree::Document::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    for draw in &scene.draws {
        let instance = scene
            .scene
            .instances
            .iter()
            .find(|i| i.name == draw.name && i.part.is_some())
            .ok_or("missing surface Part")?;
        let part = instance.part.as_ref().unwrap();
        let mut coordinates = vec![[0.0; 2]; draw.vertices.len()];
        if draw.geometry == "block" {
            if draw.vertices.len() != 24 {
                return Err("surface probe requires one quad per block face".into());
            }
            let item = doc
                .descendants()
                .find(|n| {
                    n.has_tag_name("Item") && n.attribute("referent") == Some(&instance.referent)
                })
                .ok_or("missing surface XML")?;
            let properties = item
                .children()
                .find(|n| n.has_tag_name("Properties"))
                .ok_or("missing surface properties")?;
            for (face, name) in [
                "RightSurface",
                "TopSurface",
                "BackSurface",
                "LeftSurface",
                "BottomSurface",
                "FrontSurface",
            ]
            .iter()
            .enumerate()
            {
                let surface = properties
                    .children()
                    .find(|n| n.attribute("name") == Some(*name))
                    .and_then(|n| n.text())
                    .ok_or("missing surface value")?
                    .trim()
                    .parse::<u32>()
                    .map_err(|e| e.to_string())?;
                let width = if face == 0 || face == 3 {
                    part.size[2]
                } else {
                    part.size[0]
                };
                let height = if face == 1 || face == 4 {
                    part.size[2]
                } else {
                    part.size[1]
                };
                let uv = face_uv(width, height, surface, &rows)?;
                coordinates[face * 4..face * 4 + 4].copy_from_slice(&uv);
                writeln!(
                    result.report,
                    "stud_face name={:?} face={face} surface={surface} uv={uv:?}",
                    draw.name
                )
                .unwrap();
            }
        } else if !matches!(
            draw.geometry.as_str(),
            "special_head" | "humanoid_body_mesh"
        ) {
            return Err("unsupported stud geometry".into());
        } else {
            writeln!(
                result.report,
                "stud_skip name={:?} reason=composited_file_mesh",
                draw.name
            )
            .unwrap();
        }
        if result
            .coordinates
            .insert(draw.name.clone(), coordinates)
            .is_some()
        {
            return Err("duplicate surface draw name".into());
        }
    }
    writeln!(
        result.report,
        "stud_coordinates_pass draws={} vertices={} outlines=probe_disabled",
        result.coordinates.len(),
        result.coordinates.values().map(Vec::len).sum::<usize>()
    )
    .unwrap();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surface_coordinates_tile_half_width_and_one_thirty_second_height() {
        let rows = [0.0, 0.25, 0.25, 0.0, 0.5, 0.75];
        assert_eq!(
            face_uv(2.0, 1.0, 4, &rows).unwrap(),
            [[1.0, 0.5], [0.0, 0.5], [1.0, 0.53125], [0.0, 0.53125]]
        );
        assert_eq!(face_uv(2.0, 1.0, 0, &rows).unwrap(), [[0.0; 2]; 4]);
        assert!(face_uv(2.0, 9.0, 3, &rows).is_err());
    }
}
