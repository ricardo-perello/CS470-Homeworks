//! Parse instruction strings from the input JSON into `Instruction`s, and
//! split the program into the three basic blocks (BB0 pre-loop, BB1 loop
//! body including the loop instruction, BB2 post-loop).

use crate::ir::*;
use anyhow::{anyhow, bail, Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopKind {
    Loop,
    LoopPip,
}

/// Parsed program with precomputed basic-block ranges.
#[derive(Debug, Clone)]
pub struct Program {
    pub instrs: Vec<Instruction>,
    /// Indices into `instrs`. BB0 is [0, bb1_start).
    pub bb1_start: usize,
    /// BB1 is [bb1_start, bb1_end) — INCLUDING the loop instruction.
    pub bb1_end: usize,
    /// BB2 is [bb1_end, instrs.len()).
    pub loop_kind: Option<LoopKind>,
}

impl Program {
    pub fn bb0_range(&self) -> std::ops::Range<usize> {
        0..self.bb1_start
    }
    pub fn bb1_range(&self) -> std::ops::Range<usize> {
        self.bb1_start..self.bb1_end
    }
    pub fn bb2_range(&self) -> std::ops::Range<usize> {
        self.bb1_end..self.instrs.len()
    }

    pub fn in_bb0(&self, i: InstId) -> bool {
        self.bb0_range().contains(&i)
    }
    pub fn in_bb1(&self, i: InstId) -> bool {
        self.bb1_range().contains(&i)
    }
    pub fn in_bb2(&self, i: InstId) -> bool {
        self.bb2_range().contains(&i)
    }

    /// Index of the loop / loop.pip instruction (last of BB1), if any.
    pub fn loop_idx(&self) -> Option<InstId> {
        if self.loop_kind.is_some() {
            Some(self.bb1_end - 1)
        } else {
            None
        }
    }
}

pub fn parse_program(raw: &[String]) -> Result<Program> {
    let instrs: Vec<Instruction> = raw
        .iter()
        .enumerate()
        .map(|(id, line)| parse_instruction(id, line).with_context(|| format!("at input line {id}: {line:?}")))
        .collect::<Result<_>>()?;

    // Locate the loop / loop.pip instruction (PDF guarantees at most one).
    let loop_pos = instrs
        .iter()
        .position(|i| matches!(i.opcode, Opcode::Loop | Opcode::LoopPip));

    match loop_pos {
        None => Ok(Program {
            bb1_start: instrs.len(),
            bb1_end: instrs.len(),
            loop_kind: None,
            instrs,
        }),
        Some(li) => {
            let target = instrs[li]
                .label
                .ok_or_else(|| anyhow!("loop instruction has no target address"))?;
            if target > li {
                bail!("loop target {target} is after the loop itself (idx {li})");
            }
            let kind = match instrs[li].opcode {
                Opcode::Loop => LoopKind::Loop,
                Opcode::LoopPip => LoopKind::LoopPip,
                _ => unreachable!(),
            };
            Ok(Program {
                bb1_start: target,
                bb1_end: li + 1,
                loop_kind: Some(kind),
                instrs,
            })
        }
    }
}

fn parse_instruction(id: InstId, raw: &str) -> Result<Instruction> {
    let raw = raw.trim();

    // Optional predicate prefix (pN).
    let (pred, rest) = if let Some(rest) = raw.strip_prefix('(') {
        let end = rest.find(')').ok_or_else(|| anyhow!("unclosed predicate"))?;
        let pred = parse_pred_reg(&rest[..end])?;
        (Some(pred), rest[end + 1..].trim())
    } else {
        (None, raw)
    };

    // mnemonic + operand tail
    let (mnemonic, tail) = split_mnemonic(rest);
    let operands: Vec<&str> = if tail.is_empty() {
        vec![]
    } else {
        tail.split(',').map(|s| s.trim()).collect()
    };

    let mut i = Instruction {
        id,
        opcode: Opcode::Nop,
        pred,
        dest_reg: None,
        dest_pred: None,
        dest_special: None,
        src_regs: vec![],
        imm: None,
        imm_bool: None,
        label: None,
    };

    match mnemonic {
        "add" => {
            need_operands(mnemonic, &operands, 3)?;
            i.opcode = Opcode::Add;
            i.dest_reg = Some(parse_x(operands[0])?);
            i.src_regs = vec![parse_x(operands[1])?, parse_x(operands[2])?];
        }
        "addi" => {
            need_operands(mnemonic, &operands, 3)?;
            i.opcode = Opcode::Addi;
            i.dest_reg = Some(parse_x(operands[0])?);
            i.src_regs = vec![parse_x(operands[1])?];
            i.imm = Some(parse_imm(operands[2])?);
        }
        "sub" => {
            need_operands(mnemonic, &operands, 3)?;
            i.opcode = Opcode::Sub;
            i.dest_reg = Some(parse_x(operands[0])?);
            i.src_regs = vec![parse_x(operands[1])?, parse_x(operands[2])?];
        }
        "mulu" => {
            need_operands(mnemonic, &operands, 3)?;
            i.opcode = Opcode::Mulu;
            i.dest_reg = Some(parse_x(operands[0])?);
            i.src_regs = vec![parse_x(operands[1])?, parse_x(operands[2])?];
        }
        "ld" => {
            need_operands(mnemonic, &operands, 2)?;
            i.opcode = Opcode::Ld;
            i.dest_reg = Some(parse_x(operands[0])?);
            let (imm, addr) = parse_mem(operands[1])?;
            i.imm = Some(imm);
            i.src_regs = vec![addr];
        }
        "st" => {
            need_operands(mnemonic, &operands, 2)?;
            i.opcode = Opcode::St;
            let src = parse_x(operands[0])?;
            let (imm, addr) = parse_mem(operands[1])?;
            i.src_regs = vec![src, addr];
            i.imm = Some(imm);
        }
        "loop" => {
            need_operands(mnemonic, &operands, 1)?;
            i.opcode = Opcode::Loop;
            i.label = Some(parse_imm(operands[0])? as usize);
        }
        "loop.pip" => {
            need_operands(mnemonic, &operands, 1)?;
            i.opcode = Opcode::LoopPip;
            i.label = Some(parse_imm(operands[0])? as usize);
        }
        "nop" => {
            i.opcode = Opcode::Nop;
        }
        "mov" => {
            need_operands(mnemonic, &operands, 2)?;
            let dst = operands[0];
            let src = operands[1];
            if dst.starts_with('p') {
                i.opcode = Opcode::MovPred;
                i.dest_pred = Some(parse_pred_reg(dst)?);
                i.imm_bool = Some(parse_bool(src)?);
            } else if dst == "LC" {
                i.opcode = Opcode::MovLC;
                i.dest_special = Some(SpecialReg::LC);
                i.imm = Some(parse_imm(src)?);
            } else if dst == "EC" {
                i.opcode = Opcode::MovEC;
                i.dest_special = Some(SpecialReg::EC);
                i.imm = Some(parse_imm(src)?);
            } else {
                i.dest_reg = Some(parse_x(dst)?);
                if src.starts_with('x') {
                    i.opcode = Opcode::MovReg;
                    i.src_regs = vec![parse_x(src)?];
                } else {
                    i.opcode = Opcode::MovImm;
                    i.imm = Some(parse_imm(src)?);
                }
            }
        }
        other => bail!("unknown opcode: {other}"),
    }

    Ok(i)
}

fn split_mnemonic(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

fn need_operands(op: &str, operands: &[&str], n: usize) -> Result<()> {
    if operands.len() != n {
        bail!("{op} expected {n} operands, got {}", operands.len());
    }
    Ok(())
}

fn parse_x(s: &str) -> Result<u8> {
    let s = s.trim();
    let num = s
        .strip_prefix('x')
        .ok_or_else(|| anyhow!("expected x-register, got {s:?}"))?;
    num.parse::<u8>().with_context(|| format!("parsing x-register {s:?}"))
}

fn parse_pred_reg(s: &str) -> Result<u8> {
    let s = s.trim();
    let num = s
        .strip_prefix('p')
        .ok_or_else(|| anyhow!("expected predicate register, got {s:?}"))?;
    num.parse::<u8>().with_context(|| format!("parsing predicate {s:?}"))
}

fn parse_imm(s: &str) -> Result<i64> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).with_context(|| format!("parsing hex immediate {s:?}"))
    } else if let Some(hex) = s.strip_prefix("-0x").or_else(|| s.strip_prefix("-0X")) {
        Ok(-i64::from_str_radix(hex, 16)
            .with_context(|| format!("parsing hex immediate {s:?}"))?)
    } else {
        s.parse::<i64>()
            .with_context(|| format!("parsing decimal immediate {s:?}"))
    }
}

fn parse_bool(s: &str) -> Result<bool> {
    match s.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => bail!("expected true/false, got {other:?}"),
    }
}

/// Parse `imm(xN)` form (as used by ld / st).
fn parse_mem(s: &str) -> Result<(i64, u8)> {
    let open = s
        .find('(')
        .ok_or_else(|| anyhow!("mem operand missing '(': {s:?}"))?;
    let close = s
        .find(')')
        .ok_or_else(|| anyhow!("mem operand missing ')': {s:?}"))?;
    let imm = parse_imm(s[..open].trim())?;
    let reg = parse_x(s[open + 1..close].trim())?;
    Ok((imm, reg))
}
