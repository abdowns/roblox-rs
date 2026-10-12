//! Native shader-pack records. Payloads remain borrowed from bundle bytes.
use crate::bundle::Bundle;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt::Write};

pub struct ShaderRecord<'a> {
    pub name: &'a str,
    pub digest: [u8; 16],
    pub payload: &'a [u8],
}
pub struct ShaderPack<'a> {
    pub records: BTreeMap<&'a str, ShaderRecord<'a>>,
}
impl<'a> ShaderPack<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, String> {
        if data.get(..4) != Some(b"RBXS") {
            return Err("invalid shader pack signature".into());
        }
        let word = |offset: usize| -> Result<usize, String> {
            Ok(u32::from_le_bytes(
                data.get(offset..offset + 4)
                    .ok_or("truncated shader pack")?
                    .try_into()
                    .unwrap(),
            ) as usize)
        };
        let count = word(4)?;
        let table_end = count
            .checked_mul(96)
            .and_then(|n| n.checked_add(8))
            .ok_or("shader table overflow")?;
        if count == 0 || table_end > data.len() {
            return Err("invalid shader table size".into());
        }
        let mut records = BTreeMap::new();
        let mut previous_end = table_end;
        for i in 0..count {
            let record = 8 + i * 96;
            let name_bytes = &data[record..record + 64];
            let name = std::str::from_utf8(
                &name_bytes[..name_bytes.iter().position(|b| *b == 0).unwrap_or(64)],
            )
            .map_err(|e| e.to_string())?;
            let offset = word(record + 80)?;
            let length = word(record + 84)?;
            let end = offset
                .checked_add(length)
                .ok_or("shader payload overflow")?;
            if name.is_empty() || length == 0 || offset < previous_end || end > data.len() {
                return Err("invalid shader name or payload range".into());
            }
            let digest = data[record + 64..record + 80].try_into().unwrap();
            if records
                .insert(
                    name,
                    ShaderRecord {
                        name,
                        digest,
                        payload: &data[offset..end],
                    },
                )
                .is_some()
            {
                return Err("duplicate shader name".into());
            }
            previous_end = end;
        }
        Ok(Self { records })
    }
}

pub fn inventory(bundle: &Bundle) -> Result<String, String> {
    let mut report = String::new();
    for profile in ["glsl", "glsl3", "glsles", "glsles3"] {
        let bytes = bundle.read(
            &format!("Contents/Resources/shaders/shaders_{profile}.pack"),
            64 * 1024 * 1024,
        )?;
        let pack = ShaderPack::parse(&bytes)?;
        writeln!(
            report,
            "shader_pack profile={profile} records={} bytes={} sha256={:x}",
            pack.records.len(),
            bytes.len(),
            Sha256::digest(&bytes)
        )
        .unwrap();
        for entry in pack.records.values() {
            writeln!(
                report,
                "shader_record profile={profile} name={:?} bytes={} payload_sha256={:x}",
                entry.name,
                entry.payload.len(),
                Sha256::digest(entry.payload)
            )
            .unwrap();
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shader_pack_checks_absolute_offsets_and_truncation() {
        let mut bytes = b"RBXS".to_vec();
        bytes.extend(1u32.to_le_bytes());
        bytes.resize(104, 0);
        bytes[8..13].copy_from_slice(b"Probe");
        bytes[88..92].copy_from_slice(&104u32.to_le_bytes());
        bytes[92..96].copy_from_slice(&3u32.to_le_bytes());
        bytes.extend(b"xyz");
        let pack = ShaderPack::parse(&bytes).unwrap();
        assert_eq!(pack.records["Probe"].payload, b"xyz");
        bytes[88..92].copy_from_slice(&103u32.to_le_bytes());
        assert!(ShaderPack::parse(&bytes).is_err());
        bytes[88..92].copy_from_slice(&104u32.to_le_bytes());
        bytes.pop();
        assert!(ShaderPack::parse(&bytes).is_err());
    }
}
