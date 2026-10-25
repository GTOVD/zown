//! Block descriptors for `??`, and the v0.2 value tags (decision D8).

use std::collections::HashMap;

use zown_ir::{Instr, IrProgram};

pub fn runtime() -> String {
    format!("{}{}", include_str!("v2.wat"), include_str!("v2_num.wat"))
}

/// Packed block table. `meta_base + id*4` is a u32 offset from `meta_base`
/// to that block's record: `u32 count`, then `count` items of
/// `(u32 kind, u32 a, u32 b)`.
///
/// kind 1 block id, 2 int bits, 3 string pointer, 4 name pointer, 5 float bits.
pub fn meta_image(prog: &IrProgram, lits: &HashMap<String, usize>) -> Vec<u8> {
    let n = prog.blocks.len();
    let mut recs = Vec::new();
    let mut offs = vec![0u32; n];
    for (i, block) in prog.blocks.iter().enumerate() {
        offs[i] = (n * 4 + recs.len()) as u32;
        push_u32(&mut recs, block.0.len() as u32);
        for instr in &block.0 {
            encode(instr, lits, &mut recs);
        }
    }
    let mut out = Vec::with_capacity(n * 4 + recs.len());
    for off in offs {
        push_u32(&mut out, off);
    }
    out.extend(recs);
    out
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn encode(instr: &Instr, lits: &HashMap<String, usize>, buf: &mut Vec<u8>) {
    let (kind, a, b) = match instr {
        Instr::ConstBlock(id) => (1u32, *id as u32, 0),
        Instr::ConstInt(v) => {
            let bits = *v as u64;
            (2, bits as u32, (bits >> 32) as u32)
        }
        Instr::ConstStr(s) => (3, lits[s] as u32, 0),
        Instr::Load(s) => (4, lits.get(s).copied().unwrap_or(0) as u32, 0),
        Instr::ConstFloat(v) => {
            let bits = v.to_bits();
            (5, bits as u32, (bits >> 32) as u32)
        }
        _ => (0, 0, 0),
    };
    push_u32(buf, kind);
    push_u32(buf, a);
    push_u32(buf, b);
}

pub fn intern_names(
    prog: &IrProgram,
    lits: &mut HashMap<String, usize>,
    order: &mut Vec<String>,
    cursor: &mut usize,
) {
    for block in &prog.blocks {
        for instr in &block.0 {
            let name = match instr {
                Instr::ConstCap(s) | Instr::Load(s) | Instr::Move(s) => Some(s.as_str()),
                _ => None,
            };
            if let Some(s) = name {
                if !lits.contains_key(s) {
                    lits.insert(s.to_string(), *cursor);
                    order.push(s.to_string());
                    *cursor = (*cursor + 4 + s.len() + 3) & !3;
                }
            }
        }
    }
}
