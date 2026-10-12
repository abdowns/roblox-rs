//! Independent implementation of the observed 2016 mesh v2 storage boundary.
use bevy::prelude::Component;

#[derive(Debug, Clone, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 3],
}

#[derive(Debug, Clone, Component)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<[u32; 3]>,
}

impl MeshData {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        // The native reader finds the first LF, then compares only the first 12
        // bytes of the version string. CRLF and a suffix before LF are accepted.
        let start = data
            .iter()
            .position(|b| *b == b'\n')
            .ok_or("missing mesh version newline")?
            + 1;
        if !data.starts_with(b"version 2.00") {
            return Err("only mesh version 2.00 is implemented".into());
        }
        let payload = &data[start..];
        let header = payload.get(..12).ok_or("truncated mesh header")?;
        if header[..4] != [12, 0, 36, 12] {
            return Err("invalid mesh v2 layout".into());
        }
        let count = |range: std::ops::Range<usize>| {
            u32::from_le_bytes(header[range].try_into().unwrap()) as usize
        };
        let vertices = count(4..8);
        let faces = count(8..12);
        if vertices == 0 || faces == 0 {
            return Err("empty mesh".into());
        }
        let vertex_end = vertices
            .checked_mul(36)
            .and_then(|v| v.checked_add(12))
            .ok_or("mesh size overflow")?;
        let end = faces
            .checked_mul(12)
            .and_then(|v| v.checked_add(vertex_end))
            .ok_or("mesh size overflow")?;
        if end != payload.len() {
            return Err("mesh byte count mismatch".into());
        }
        // Research resource limit; checked before allocation. This is not a native limit.
        if data.len() > 64 * 1024 * 1024 {
            return Err("mesh exceeds 64 MiB research limit".into());
        }
        let mut result = Self {
            vertices: Vec::with_capacity(vertices),
            triangles: Vec::with_capacity(faces),
        };
        for record in payload[12..vertex_end].chunks_exact(36) {
            let mut values = [0.0; 9];
            for (i, bytes) in record.chunks_exact(4).enumerate() {
                values[i] = f32::from_le_bytes(bytes.try_into().unwrap());
            }
            result.vertices.push(Vertex {
                position: values[0..3].try_into().unwrap(),
                normal: values[3..6].try_into().unwrap(),
                uv: values[6..9].try_into().unwrap(),
            });
        }
        for record in payload[vertex_end..].chunks_exact(12) {
            let face: [u32; 3] = std::array::from_fn(|i| {
                u32::from_le_bytes(record[i * 4..i * 4 + 4].try_into().unwrap())
            });
            if face.iter().any(|i| *i as usize >= vertices) {
                return Err("mesh face index out of range".into());
            }
            result.triangles.push(face);
        }
        Ok(result)
    }

    /// Diagnostic validation is deliberately separate from native byte decoding.
    pub fn finite_bounds(&self) -> Result<([f32; 3], [f32; 3]), String> {
        let mut low = [f32::INFINITY; 3];
        let mut high = [f32::NEG_INFINITY; 3];
        if self.vertices.is_empty() {
            return Err("empty diagnostic mesh".into());
        }
        for vertex in &self.vertices {
            if vertex
                .position
                .iter()
                .chain(&vertex.normal)
                .chain(&vertex.uv)
                .any(|f| !f.is_finite())
            {
                return Err("non-finite mesh diagnostic input".into());
            }
            for axis in 0..3 {
                low[axis] = low[axis].min(vertex.position[axis]);
                high[axis] = high[axis].max(vertex.position[axis]);
            }
        }
        Ok((low, high))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn triangle() -> Vec<u8> {
        let mut bytes = b"version 2.00\n".to_vec();
        bytes.extend([12, 0, 36, 12]);
        bytes.extend(3u32.to_le_bytes());
        bytes.extend(1u32.to_le_bytes());
        for pos in [[-2.0f32, 1.0, 3.0], [4.0, 2.0, 1.0], [1.0, -3.0, -1.0]] {
            for value in pos.into_iter().chain([0.0, 1.0, 0.0, 0.25, 0.75, -0.0]) {
                bytes.extend(value.to_le_bytes());
            }
        }
        for i in [2u32, 0, 1] {
            bytes.extend(i.to_le_bytes());
        }
        bytes
    }
    #[test]
    fn decodes_native_record_order_and_float_bits() {
        let mesh = MeshData::parse(&triangle()).unwrap();
        assert_eq!(mesh.vertices[1].position, [4.0, 2.0, 1.0]);
        assert_eq!(mesh.vertices[0].normal, [0.0, 1.0, 0.0]);
        assert_eq!(mesh.vertices[0].uv[2].to_bits(), (-0.0f32).to_bits());
        assert_eq!(mesh.triangles, [[2, 0, 1]]);
        assert_eq!(
            mesh.finite_bounds().unwrap(),
            ([-2.0, -3.0, -1.0], [4.0, 2.0, 3.0])
        );
    }
    #[test]
    fn follows_native_newline_and_twelve_byte_version_comparison() {
        let data = triangle();
        for prefix in [b"version 2.00\r\n".as_slice(), b"version 2.00 suffix\n"] {
            let mut changed = prefix.to_vec();
            changed.extend(&data[13..]);
            assert_eq!(MeshData::parse(&changed).unwrap().triangles, [[2, 0, 1]]);
        }
        assert!(MeshData::parse(&data[13..]).is_err());
    }
    #[test]
    fn rejects_truncation_layout_empty_and_out_of_range_faces() {
        let original = triangle();
        for n in 0..original.len() {
            assert!(MeshData::parse(&original[..n]).is_err());
        }
        for (offset, value) in [(13, 11u8), (15, 35), (16, 11)] {
            let mut data = original.clone();
            data[offset] = value;
            assert!(MeshData::parse(&data).is_err());
        }
        let mut data = original.clone();
        data[17..21].fill(0);
        assert!(MeshData::parse(&data).is_err());
        let mut data = original.clone();
        let length = data.len();
        data[length - 4..].copy_from_slice(&3u32.to_le_bytes());
        assert!(MeshData::parse(&data).is_err());
        let mut data = original;
        data.push(0);
        assert!(MeshData::parse(&data).is_err());
    }
    #[test]
    fn native_decode_preserves_nan_while_diagnostic_rejects_it() {
        let mut data = triangle();
        let bits = 0x7fc01234u32;
        data[25..29].copy_from_slice(&bits.to_le_bytes());
        let mesh = MeshData::parse(&data).unwrap();
        assert_eq!(mesh.vertices[0].position[0].to_bits(), bits);
        assert!(mesh.finite_bounds().is_err());
    }
}
