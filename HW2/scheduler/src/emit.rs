//! Serialize a `Schedule` into the reference JSON format: an array of
//! bundles, each bundle an array of exactly 5 strings in the order
//! `[ALU0, ALU1, Mult, Mem, Branch]`. Unused slots become `"nop"`.

use crate::ir::*;

pub fn emit(schedule: &Schedule) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = schedule
        .flat()
        .map(|bundle| {
            ExecUnit::ALL
                .iter()
                .map(|&u| match bundle.get(u) {
                    Some(i) => format_instruction(i),
                    None => "nop".to_string(),
                })
                .collect()
        })
        .collect();

    // Trim any trailing fully-empty bundles to match reference format.
    while out
        .last()
        .is_some_and(|b| b.iter().all(|s| s == " nop" || s == "nop"))
    {
        out.pop();
    }

    out
}

fn format_instruction(i: &Instruction) -> String {
    use Opcode::*;
    let pred = i
        .pred
        .map(|p| format!("(p{p}) "))
        .unwrap_or_default();
    let body = match i.opcode {
        Add => format!(
            "add x{}, x{}, x{}",
            i.dest_reg.unwrap(),
            i.src_regs[0],
            i.src_regs[1]
        ),
        Addi => format!(
            "addi x{}, x{}, {}",
            i.dest_reg.unwrap(),
            i.src_regs[0],
            i.imm.unwrap()
        ),
        Sub => format!(
            "sub x{}, x{}, x{}",
            i.dest_reg.unwrap(),
            i.src_regs[0],
            i.src_regs[1]
        ),
        Mulu => format!(
            "mulu x{}, x{}, x{}",
            i.dest_reg.unwrap(),
            i.src_regs[0],
            i.src_regs[1]
        ),
        Ld => format!(
            "ld x{}, {}(x{})",
            i.dest_reg.unwrap(),
            i.imm.unwrap(),
            i.src_regs[0]
        ),
        St => format!(
            "st x{}, {}(x{})",
            i.src_regs[0],
            i.imm.unwrap(),
            i.src_regs[1]
        ),
        Loop => format!("loop {}", i.label.unwrap()),
        LoopPip => format!("loop.pip {}", i.label.unwrap()),
        Nop => "nop".to_string(),
        MovReg => format!("mov x{}, x{}", i.dest_reg.unwrap(), i.src_regs[0]),
        MovImm => format!("mov x{}, {}", i.dest_reg.unwrap(), i.imm.unwrap()),
        MovPred => format!(
            "mov p{}, {}",
            i.dest_pred.unwrap(),
            if i.imm_bool.unwrap() { "true" } else { "false" }
        ),
        MovLC => format!("mov LC, {}", i.imm.unwrap()),
        MovEC => format!("mov EC, {}", i.imm.unwrap()),
    };
    // The reference JSON formats unpredicated slots with a leading space.
    if pred.is_empty() {
        format!(" {body}")
    } else {
        format!("{pred}{body}")
    }
}

