use anyhow::{anyhow, bail, Context};

use crate::types::{Instruction, OpCode, OperandB};

fn parse_reg(tok: &str) -> anyhow::Result<u8> {
    let t = tok.trim();
    let t = t.strip_prefix('x').ok_or_else(|| anyhow!("bad register token: {tok}"))?;
    let v: u8 = t.parse().with_context(|| format!("bad register number: {tok}"))?;
    if v > 31 {
        bail!("register out of range: {tok}");
    }
    Ok(v)
}

fn parse_imm(tok: &str) -> anyhow::Result<i64> {
    let t = tok.trim();
    let v: i64 = t.parse().with_context(|| format!("bad immediate: {tok}"))?;
    Ok(v)
}

fn parse_line(pc: u64, line: &str) -> anyhow::Result<Instruction> {
    let s = line.trim();
    if s.is_empty() {
        bail!("empty instruction string");
    }

    // Expected forms:
    // add  xD, xA, xB
    // sub  xD, xA, xB
    // mulu xD, xA, xB
    // divu xD, xA, xB
    // remu xD, xA, xB
    // addi xD, xA, imm
    let mut parts = s.splitn(2, char::is_whitespace);
    let op_str = parts.next().unwrap().trim();
    let rest = parts.next().unwrap_or("").trim();

    let args: Vec<&str> = rest.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();

    match op_str {
        "add" | "sub" | "mulu" | "divu" | "remu" => {
            if args.len() != 3 {
                bail!("bad operand count for {op_str}: {line}");
            }
            let dest = parse_reg(args[0])?;
            let op_a = parse_reg(args[1])?;
            let op_b = parse_reg(args[2])?;
            let op = match op_str {
                "add" => OpCode::Add,
                "sub" => OpCode::Sub,
                "mulu" => OpCode::Mulu,
                "divu" => OpCode::Divu,
                "remu" => OpCode::Remu,
                _ => unreachable!(),
            };
            Ok(Instruction {
                pc,
                op,
                dest,
                op_a,
                op_b: OperandB::Reg(op_b),
            })
        }
        "addi" => {
            if args.len() != 3 {
                bail!("bad operand count for addi: {line}");
            }
            let dest = parse_reg(args[0])?;
            let op_a = parse_reg(args[1])?;
            let imm = parse_imm(args[2])?;
            Ok(Instruction {
                pc,
                op: OpCode::Add, // addi modeled as add with imm as operand B
                dest,
                op_a,
                op_b: OperandB::Imm(imm),
            })
        }
        _ => bail!("unknown opcode: {op_str}"),
    }
}

pub fn parse_program(json_text: &str) -> anyhow::Result<Vec<Instruction>> {
    let raw: Vec<String> = serde_json::from_str(json_text).context("input JSON must be an array of strings")?;
    let mut out = Vec::with_capacity(raw.len());
    for (i, s) in raw.iter().enumerate() {
        out.push(parse_line(i as u64, s)?);
    }
    Ok(out)
}

