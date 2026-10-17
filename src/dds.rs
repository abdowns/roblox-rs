//! Bounded DDS records (DXT1, DXT5 and uncompressed L8A8) and recovered squish
//! decompression.
use crate::texture::{Mip, PngTexture};

pub struct Bc1Dds<'a> {
    pub width: u32,
    pub height: u32,
    pub mips: Vec<&'a [u8]>,
}
/// A legacy DXT5 (BC3) DDS: the DXT1 header with FourCC "DXT5" and 16-byte
/// blocks (8 alpha bytes, then a color block).
pub struct Bc3Dds<'a> {
    pub width: u32,
    pub height: u32,
    pub mips: Vec<&'a [u8]>,
}

/// The shared DDS header checks: a 124-byte header, a 32-byte pixel format
/// with the given FourCC, no cube or volume caps and a mip chain that fits
/// the file exactly.
fn parse_blocks<'a>(
    bytes: &'a [u8],
    fourcc: &[u8; 4],
    block: usize,
) -> Result<(u32, u32, Vec<&'a [u8]>), String> {
    let word = |at| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .ok_or("truncated DDS".to_string())
    };
    if bytes.get(..4) != Some(b"DDS ")
        || word(4)? != 124
        || word(76)? != 32
        || bytes.get(84..88) != Some(fourcc)
    {
        return Err(format!(
            "DDS probe requires a legacy {} header",
            String::from_utf8_lossy(fourcc)
        ));
    }
    let height = word(12)?;
    let width = word(16)?;
    let count = word(28)?.max(1);
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || word(112)? != 0
        || count > (32 - width.max(height).leading_zeros())
    {
        return Err("unsupported DDS dimensions, cube/volume or mip count".into());
    }
    let mut at = 128;
    let mut mips = Vec::new();
    for level in 0..count {
        let length = (width >> level).max(1).div_ceil(4) as usize
            * (height >> level).max(1).div_ceil(4) as usize
            * block;
        let end = at + length;
        mips.push(bytes.get(at..end).ok_or("truncated DDS mip")?);
        at = end;
    }
    if at != bytes.len() {
        return Err("unexpected trailing DDS payload".into());
    }
    Ok((width, height, mips))
}

/// Expand 4x4 blocks into RGBA mips.
fn decode_mips(
    width: u32,
    height: u32,
    mips: &[&[u8]],
    block: usize,
    decode: impl Fn(&[u8]) -> [[u8; 4]; 16],
) -> PngTexture {
    let mut result = Vec::new();
    for (level, bytes) in mips.iter().enumerate() {
        let width = (width >> level).max(1);
        let height = (height >> level).max(1);
        let mut rgba = vec![0; width as usize * height as usize * 4];
        for (index, data) in bytes.chunks_exact(block).enumerate() {
            let x = index as u32 % width.div_ceil(4) * 4;
            let y = index as u32 / width.div_ceil(4) * 4;
            let decoded = decode(data);
            for dy in 0..4 {
                for dx in 0..4 {
                    if x + dx < width && y + dy < height {
                        let target = (((y + dy) * width + x + dx) * 4) as usize;
                        rgba[target..target + 4].copy_from_slice(&decoded[(dy * 4 + dx) as usize]);
                    }
                }
            }
        }
        result.push(Mip {
            width,
            height,
            rgba,
        });
    }
    PngTexture { mips: result }
}

impl<'a> Bc1Dds<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        let (width, height, mips) = parse_blocks(bytes, b"DXT1", 8)?;
        Ok(Self {
            width,
            height,
            mips,
        })
    }

    /// Native Image::load forced-uncompressed branch, not a modern codec default.
    pub fn decode(&self) -> PngTexture {
        decode_mips(self.width, self.height, &self.mips, 8, |b| {
            decode_block(b.try_into().unwrap())
        })
    }
}

impl<'a> Bc3Dds<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        let (width, height, mips) = parse_blocks(bytes, b"DXT5", 16)?;
        Ok(Self {
            width,
            height,
            mips,
        })
    }

    /// squish DecompressAlphaDxt5 then DecompressColour(isDxt1 = false):
    /// the color block always uses the four-color palette.
    pub fn decode(&self) -> PngTexture {
        decode_mips(self.width, self.height, &self.mips, 16, |b| {
            let mut colors = decode_color(b[8..].try_into().unwrap(), false);
            let alpha = decode_alpha(b[..8].try_into().unwrap());
            for (color, a) in colors.iter_mut().zip(alpha) {
                color[3] = a;
            }
            colors
        })
    }
}

/// An uncompressed DDS with an 8-bit luminance and 8-bit alpha channel
/// (pixel-format flags 0x20001, 16 bits, masks 0xff / 0xff00), expanded as
/// GL_LUMINANCE_ALPHA samples: (L, L, L, A). Only the first level is read.
pub fn luminance_alpha(bytes: &[u8]) -> Result<PngTexture, String> {
    let word = |at| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .ok_or("truncated DDS".to_string())
    };
    if bytes.get(..4) != Some(b"DDS ")
        || word(4)? != 124
        || word(76)? != 32
        || word(80)? != 0x20001
        || word(88)? != 16
        || word(92)? != 0xff
        || word(104)? != 0xff00
        || word(112)? != 0
    {
        return Err("DDS probe requires an uncompressed L8A8 header".into());
    }
    let height = word(12)?;
    let width = word(16)?;
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("unsupported L8A8 dimensions".into());
    }
    let length = width as usize * height as usize * 2;
    let pixels = bytes.get(128..128 + length).ok_or("truncated L8A8 DDS")?;
    let rgba = pixels
        .chunks_exact(2)
        .flat_map(|p| [p[0], p[0], p[0], p[1]])
        .collect();
    Ok(PngTexture {
        mips: vec![Mip {
            width,
            height,
            rgba,
        }],
    })
}

/// squish DecompressAlphaDxt5: two endpoints, then 16 3-bit indices. a0 > a1
/// interpolates six values ((7 - i) a0 + i a1) / 7; otherwise four values
/// ((5 - i) a0 + i a1) / 5 followed by 0 and 255. Integer division, as squish.
fn decode_alpha(bytes: &[u8; 8]) -> [u8; 16] {
    let (a0, a1) = (u32::from(bytes[0]), u32::from(bytes[1]));
    let mut codes = [0u8; 8];
    codes[0] = a0 as u8;
    codes[1] = a1 as u8;
    if a0 <= a1 {
        for i in 1..5 {
            codes[1 + i] = (((5 - i as u32) * a0 + i as u32 * a1) / 5) as u8;
        }
        codes[6] = 0;
        codes[7] = 255;
    } else {
        for i in 1..7 {
            codes[1 + i] = (((7 - i as u32) * a0 + i as u32 * a1) / 7) as u8;
        }
    }
    let mut bits = 0u64;
    for (i, b) in bytes[2..].iter().enumerate() {
        bits |= u64::from(*b) << (8 * i);
    }
    std::array::from_fn(|i| codes[((bits >> (3 * i)) & 7) as usize])
}

fn decode_block(bytes: &[u8; 8]) -> [[u8; 4]; 16] {
    decode_color(bytes, true)
}

fn decode_color(bytes: &[u8; 8], dxt1: bool) -> [[u8; 4]; 16] {
    let a = u16::from_le_bytes(bytes[..2].try_into().unwrap());
    let b = u16::from_le_bytes(bytes[2..4].try_into().unwrap());
    let expand = |color: u16| {
        let r = (color >> 11) & 31;
        let g = (color >> 5) & 63;
        let b = color & 31;
        [
            ((r << 3) | (r >> 2)) as u8,
            ((g << 2) | (g >> 4)) as u8,
            ((b << 3) | (b >> 2)) as u8,
            255,
        ]
    };
    let first = expand(a);
    let second = expand(b);
    let third = std::array::from_fn(|c| {
        if c == 3 {
            255
        } else if dxt1 && a <= b {
            ((first[c] as u16 + second[c] as u16) / 2) as u8
        } else {
            ((first[c] as u16 * 2 + second[c] as u16) / 3) as u8
        }
    });
    let fourth = std::array::from_fn(|c| {
        if dxt1 && a <= b {
            0
        } else if c == 3 {
            255
        } else {
            ((first[c] as u16 + second[c] as u16 * 2) / 3) as u8
        }
    });
    let palette = [first, second, third, fourth];
    let indices = u32::from_le_bytes(bytes[4..].try_into().unwrap());
    std::array::from_fn(|i| palette[((indices >> (i * 2)) & 3) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dxt1_preserves_native_endpoint_order_and_alpha_branch() {
        let colors = decode_block(&[0, 248, 31, 0, 0xe4, 0xe4, 0xe4, 0xe4]);
        assert_eq!(
            &colors[..4],
            &[
                [255, 0, 0, 255],
                [0, 0, 255, 255],
                [170, 0, 85, 255],
                [85, 0, 170, 255]
            ]
        );
        let colors = decode_block(&[31, 0, 0, 248, 0xe4, 0xe4, 0xe4, 0xe4]);
        assert_eq!(
            &colors[..4],
            &[
                [0, 0, 255, 255],
                [255, 0, 0, 255],
                [127, 0, 127, 255],
                [0, 0, 0, 0]
            ]
        );
    }
    #[test]
    fn dxt5_alpha_follows_squish_integer_palettes() {
        // a0 > a1: eight values; index 2 = (6*200 + 1*100) / 7 = 185.
        let mut block = [200, 100, 0, 0, 0, 0, 0, 0];
        block[2] = 0b010_001_000; // indices 0, 1, 2 for the first three pixels
        let a = decode_alpha(&block);
        assert_eq!(&a[..3], &[200, 100, 185]);
        // a0 <= a1: six values plus 0 and 255.
        let block = [10, 20, 0b110_010_111u16 as u8, 0b1, 0, 0, 0, 0];
        let a = decode_alpha(&block);
        assert_eq!(&a[..3], &[255, 12, 0]);
        // DXT5 color blocks always use the four-color palette.
        let colors = decode_color(&[31, 0, 0, 248, 0xe4, 0xe4, 0xe4, 0xe4], false);
        assert_eq!(colors[3], [170, 0, 85, 255]);
    }
    #[test]
    fn dds_handles_partial_blocks_and_rejects_truncation() {
        let mut b = vec![0; 128];
        b[..4].copy_from_slice(b"DDS ");
        for (at, value) in [(4, 124u32), (12, 1), (16, 1), (28, 1), (76, 32)] {
            b[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        b[84..88].copy_from_slice(b"DXT1");
        b.extend([0, 248, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            Bc1Dds::parse(&b).unwrap().decode().mips[0].rgba,
            [255, 0, 0, 255]
        );
        b.pop();
        assert!(Bc1Dds::parse(&b).is_err());
    }
}
