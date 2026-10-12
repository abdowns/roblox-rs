//! Read rendering tables from the verified image, never embed extracted tables.
use crate::bundle::{Bundle, verify_mach_header};
use iced_x86::{Decoder, DecoderOptions, Mnemonic, OpKind, Register};
use std::collections::{BTreeMap, HashMap};

pub struct NativeImage {
    bytes: Vec<u8>,
}

impl NativeImage {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let bytes = bundle.read("Contents/MacOS/RobloxStudio", 64 * 1024 * 1024)?;
        verify_mach_header(&bytes)?;
        Ok(Self { bytes })
    }

    pub fn region(&self, address: u32, length: usize) -> Result<&[u8], String> {
        let word = |at: usize| -> Result<u32, String> {
            Ok(u32::from_le_bytes(
                self.bytes
                    .get(at..at + 4)
                    .ok_or("truncated Mach-O")?
                    .try_into()
                    .unwrap(),
            ))
        };
        let mut at = 28;
        for _ in 0..word(16)? {
            let command = word(at)?;
            let size = word(at + 4)? as usize;
            if size < 8 {
                return Err("invalid Mach-O load command".into());
            }
            if command == 1 {
                let base = word(at + 24)?;
                let offset = word(at + 32)? as usize;
                let file_size = word(at + 36)? as usize;
                if let Some(delta) = address.checked_sub(base) {
                    let delta = delta as usize;
                    if delta
                        .checked_add(length)
                        .is_some_and(|end| end <= file_size)
                    {
                        let begin = offset.checked_add(delta).ok_or("Mach-O offset overflow")?;
                        return self
                            .bytes
                            .get(begin..begin + length)
                            .ok_or_else(|| "Mach-O region outside file".into());
                    }
                }
            }
            at = at.checked_add(size).ok_or("Mach-O command overflow")?;
        }
        Err(format!("unmapped file-backed address {address:08x}"))
    }

    /// Recover constant arguments to the 208 color registrations. This decodes
    /// instructions; it never runs x86 code or attempts general emulation.
    pub fn brick_colors(&self) -> Result<BTreeMap<u32, [u8; 3]>, String> {
        let code = self.region(0x00650222, 28422)?;
        let colors = decode_colors(code, 0x00650222, 0x00659b66)?;
        if colors.len() != 208 {
            return Err(format!(
                "expected 208 native colors, recovered {}",
                colors.len()
            ));
        }
        Ok(colors)
    }

    pub fn block_faces(&self) -> Result<[[usize; 4]; 6], String> {
        let data = self.region(0x017f0140, 120)?;
        let faces = std::array::from_fn(|face| {
            std::array::from_fn(|corner| {
                let at = face * 20 + corner * 4;
                u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize
            })
        });
        if faces.iter().flatten().any(|v| *v > 7) {
            return Err("invalid native block face table".into());
        }
        Ok(faces)
    }

    pub fn surface_rows(&self) -> Result<[f32; 6], String> {
        let bytes = self.region(0x017f045c, 24)?;
        let rows = std::array::from_fn(|i| {
            f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
        });
        if rows
            .iter()
            .any(|v| !v.is_finite() || !(0.0..1.0).contains(v))
        {
            return Err("invalid native surface row table".into());
        }
        Ok(rows)
    }
}

fn decode_colors(code: &[u8], base: u64, setter: u64) -> Result<BTreeMap<u32, [u8; 3]>, String> {
    let mut decoder = Decoder::with_ip(32, code, base, DecoderOptions::NONE);
    let mut registers = HashMap::<Register, u32>::new();
    let mut stack = HashMap::<u64, u32>::new();
    let mut colors = BTreeMap::new();
    while decoder.can_decode() {
        let i = decoder.decode();
        if i.is_invalid() {
            return Err("invalid palette initializer instruction".into());
        }
        if i.mnemonic() == Mnemonic::Mov {
            let value = match i.op1_kind() {
                OpKind::Immediate32 => Some(i.immediate32()),
                OpKind::Immediate8to32 => Some(i.immediate8to32() as u32),
                OpKind::Register => registers.get(&i.op1_register()).copied(),
                _ => None,
            };
            if i.op0_kind() == OpKind::Register {
                registers.remove(&i.op0_register());
                if let Some(v) = value {
                    registers.insert(i.op0_register(), v);
                }
            } else if i.op0_kind() == OpKind::Memory
                && i.memory_base() == Register::ESP
                && i.memory_index() == Register::None
            {
                stack.remove(&i.memory_displacement64());
                if let Some(v) = value {
                    stack.insert(i.memory_displacement64(), v);
                }
            }
        } else if i.mnemonic() == Mnemonic::Xor
            && i.op0_kind() == OpKind::Register
            && i.op1_kind() == OpKind::Register
            && i.op0_register() == i.op1_register()
        {
            registers.insert(i.op0_register(), 0);
        } else if i.mnemonic() == Mnemonic::Call {
            if i.op0_kind() == OpKind::NearBranch32 && i.near_branch_target() == setter {
                let args: Result<Vec<_>, _> = [4, 8, 12, 16]
                    .into_iter()
                    .map(|offset| {
                        stack
                            .get(&offset)
                            .copied()
                            .ok_or("unresolved color argument")
                    })
                    .collect();
                let a = args?;
                let rgb: [u8; 3] = std::array::from_fn(|axis| a[axis + 1] as u8);
                if a[1..].iter().any(|v| *v > 255) || colors.insert(a[0], rgb).is_some() {
                    return Err("invalid or duplicate color registration".into());
                }
            }
            for register in [Register::EAX, Register::ECX, Register::EDX] {
                registers.remove(&register);
            }
            stack.clear();
        } else if i.op_count() > 0 && i.op0_kind() == OpKind::Register {
            // All other register writes are unknown to this bounded evaluator.
            registers.remove(&i.op0_register());
        }
    }
    Ok(colors)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initializer_recovers_immediate_and_zero_register_arguments() {
        let mut code = vec![0x31, 0xc0, 0x89, 0x44, 0x24, 0x10, 0x89, 0x44, 0x24, 0x0c];
        for (offset, v) in [(8u8, 217u32), (4, 73)] {
            code.extend([0xc7, 0x44, 0x24, offset]);
            code.extend(v.to_le_bytes());
        }
        let target = 0x2000u64;
        let relative = (target - (0x1000 + code.len() as u64 + 5)) as u32;
        code.push(0xe8);
        code.extend(relative.to_le_bytes());
        assert_eq!(
            decode_colors(&code, 0x1000, target).unwrap()[&73],
            [217, 0, 0]
        );
        code[0..2].copy_from_slice(&[0x90, 0x90]);
        assert!(decode_colors(&code, 0x1000, target).is_err());
    }
}
