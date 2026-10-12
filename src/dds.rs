//! Bounded DDS/DXT1 records and recovered squish color decompression.
use crate::texture::{Mip, PngTexture};

pub struct Bc1Dds<'a> {
    pub width: u32,
    pub height: u32,
    pub mips: Vec<&'a [u8]>,
}
impl<'a> Bc1Dds<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        let word = |at| {
            bytes
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .ok_or("truncated DDS".to_string())
        };
        if bytes.get(..4) != Some(b"DDS ")
            || word(4)? != 124
            || word(76)? != 32
            || bytes.get(84..88) != Some(b"DXT1")
        {
            return Err("DDS probe requires a legacy DXT1 header".into());
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
                * 8;
            let end = at + length;
            mips.push(bytes.get(at..end).ok_or("truncated DXT1 mip")?);
            at = end;
        }
        if at != bytes.len() {
            return Err("unexpected trailing DDS payload".into());
        }
        Ok(Self {
            width,
            height,
            mips,
        })
    }

    /// Native Image::load forced-uncompressed branch, not a modern codec default.
    pub fn decode(&self) -> PngTexture {
        let mut mips = Vec::new();
        for (level, bytes) in self.mips.iter().enumerate() {
            let width = (self.width >> level).max(1);
            let height = (self.height >> level).max(1);
            let mut rgba = vec![0; width as usize * height as usize * 4];
            for (index, block) in bytes.chunks_exact(8).enumerate() {
                let x = index as u32 % width.div_ceil(4) * 4;
                let y = index as u32 / width.div_ceil(4) * 4;
                let decoded = decode_block(block.try_into().unwrap());
                for dy in 0..4 {
                    for dx in 0..4 {
                        if x + dx < width && y + dy < height {
                            let target = (((y + dy) * width + x + dx) * 4) as usize;
                            rgba[target..target + 4]
                                .copy_from_slice(&decoded[(dy * 4 + dx) as usize]);
                        }
                    }
                }
            }
            mips.push(Mip {
                width,
                height,
                rgba,
            });
        }
        PngTexture { mips }
    }
}

fn decode_block(bytes: &[u8; 8]) -> [[u8; 4]; 16] {
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
        } else if a <= b {
            ((first[c] as u16 + second[c] as u16) / 2) as u8
        } else {
            ((first[c] as u16 * 2 + second[c] as u16) / 3) as u8
        }
    });
    let fourth = std::array::from_fn(|c| {
        if a <= b {
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
