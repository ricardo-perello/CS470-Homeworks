//! Register allocation for the `loop` (non-pipelined) schedule.
//! Follows PDF §3.3.1 exactly:
//!
//!   * **Phase 1** — Assign fresh unique x-registers (starting at x1) to each
//!     producing instruction, walking the schedule in *scheduling order*:
//!     bundle 0 slot [Alu0, Alu1, Mult, Mem, Branch], then bundle 1, etc.
//!
//!   * **Phase 2** — Rewrite each source operand to read from its producer's
//!     newly assigned register. Interloop deps read the *BB0 initializer's*
//!     new register (consumers see a "loop function argument"; the actual
//!     per-iteration value comes from the mov inserted in phase 3).
//!
//!   * **Phase 3** — Insert a `mov` at the end of BB1 for each interloop
//!     dep, copying the BB1 producer's new register back into the BB0
//!     initializer's new register (so the "function argument" holds the
//!     right value before the next iteration). Movs are placed at the latest
//!     bundle ≤ the loop bundle that is ≥ `producer.bundle + latency`,
//!     preferring Alu0 then Alu1. If no slot fits, push the loop down by one
//!     bundle and retry. Multiple movs claim distinct bundles — walking
//!     backward from the loop bundle ensures each gets its own Alu0 slot
//!     when possible (this matches the reference outputs, e.g. test 17).
//!
//!   * **Phase 4** — For every source operand whose dependency analysis
//!     returned `None` (i.e. no producer wrote it), assign a fresh "unused"
//!     x-register. One fresh register *per operand occurrence*, walking the
//!     schedule in scheduling order.

use std::collections::{HashMap, HashSet};

use crate::ir::*;
use crate::parser::Program;

pub fn allocate(
    schedule: &mut Schedule,
    program: &Program,
    deps: &DepTable,
    placement: &HashMap<InstId, Placement>,
) {
    // ---- Phase 1: assign fresh regs to producers ----
    let mut new_reg: HashMap<InstId, u8> = HashMap::new();
    let mut next_reg: u8 = 1;
    for bidx in 0..schedule.total_bundles() {
        for unit in ExecUnit::ALL {
            if let Some(instr) = schedule.bundle(bidx).get(unit) {
                if instr.dest_reg.is_some() {
                    new_reg.insert(instr.id, next_reg);
                    next_reg += 1;
                }
            }
        }
    }

    // Write phase-1 renames into the scheduled instructions.
    for bidx in 0..schedule.total_bundles() {
        for unit in ExecUnit::ALL {
            let bundle = schedule.bundle_mut(bidx);
            if let Some(instr) = bundle.get_mut(unit) {
                if let Some(&r) = new_reg.get(&instr.id) {
                    instr.dest_reg = Some(r);
                }
            }
        }
    }

    // ---- Phase 2: rewrite source operands ----
    for bidx in 0..schedule.total_bundles() {
        for unit in ExecUnit::ALL {
            let bundle = schedule.bundle_mut(bidx);
            let Some(instr) = bundle.get_mut(unit) else {
                continue;
            };
            if instr.id >= deps.len() {
                continue;
            }
            let id = instr.id;
            for (i, dep) in deps[id].src_deps.iter().enumerate() {
                let producer = match dep {
                    Some(DepKind::Local(p))
                    | Some(DepKind::LoopInvariant(p))
                    | Some(DepKind::PostLoop(p)) => Some(*p),
                    Some(DepKind::Interloop { bb0, bb1 }) => Some(bb0.unwrap_or(*bb1)),
                    None => None,
                };
                if let Some(p) = producer {
                    if let Some(&nr) = new_reg.get(&p) {
                        if i < instr.src_regs.len() {
                            instr.src_regs[i] = nr;
                        }
                    }
                }
            }
        }
    }

    // ---- Phase 3: insert interloop movs ----
    if let Some(loop_idx) = program.loop_idx() {
        // Unique (bb0 producer, bb1 producer) pairs with a BB0 initializer.
        let mut pairs: HashSet<(InstId, InstId)> = HashSet::new();
        for inst_deps in deps {
            for dep in &inst_deps.src_deps {
                if let Some(DepKind::Interloop {
                    bb0: Some(p0),
                    bb1: p1,
                }) = dep
                {
                    pairs.insert((*p0, *p1));
                }
            }
        }

        // Attach min_cycle = bb1_producer.bundle + latency(bb1_producer).
        let mut movs: Vec<(InstId, InstId, usize)> = pairs
            .into_iter()
            .map(|(p0, p1)| {
                let min = placement[&p1].bundle + program.instrs[p1].latency();
                (p0, p1, min)
            })
            .collect();

        // Descending min_cycle — largest-constraint first claims the loop bundle.
        movs.sort_by(|a, b| b.2.cmp(&a.2));

        let mut mov_id_counter = program.instrs.len();
        for (p0, p1, min_cycle) in movs {
            let src = new_reg[&p1];
            let dst = new_reg[&p0];
            place_mov(schedule, loop_idx, min_cycle, src, dst, mov_id_counter);
            mov_id_counter += 1;
        }
    }

    // ---- Phase 4: unused regs per operand occurrence, in scheduling order ----
    for bidx in 0..schedule.total_bundles() {
        for unit in ExecUnit::ALL {
            let bundle = schedule.bundle_mut(bidx);
            let Some(instr) = bundle.get_mut(unit) else {
                continue;
            };
            if instr.id >= deps.len() {
                continue; // skip nops and inserted movs
            }
            let id = instr.id;
            for (i, dep) in deps[id].src_deps.iter().enumerate() {
                if dep.is_none() && i < instr.src_regs.len() {
                    instr.src_regs[i] = next_reg;
                    next_reg += 1;
                }
            }
        }
    }
}

/// Place a single interloop mov, pushing the loop down if no slot fits.
fn place_mov(
    schedule: &mut Schedule,
    loop_idx: InstId,
    min_cycle: usize,
    src: u8,
    dst: u8,
    mov_id: InstId,
) {
    loop {
        let loop_bundle = find_loop_bundle(schedule, loop_idx);
        if let Some((b, u)) = find_latest_alu_slot(schedule, min_cycle, loop_bundle) {
            schedule
                .bundle_mut(b)
                .set(u, make_mov_reg(dst, src, mov_id));
            return;
        }
        push_loop_down(schedule, loop_idx);
    }
}

fn find_loop_bundle(schedule: &Schedule, loop_idx: InstId) -> usize {
    let base = schedule.bb0.len();
    for (i, b) in schedule.bb1.iter().enumerate() {
        for u in ExecUnit::ALL {
            if let Some(instr) = b.get(u) {
                if instr.id == loop_idx {
                    return base + i;
                }
            }
        }
    }
    unreachable!("loop instruction missing from bb1")
}

/// Walk `[lo, hi]` from `hi` down. Prefer Alu0; fall back to Alu1.
fn find_latest_alu_slot(
    schedule: &Schedule,
    lo: usize,
    hi: usize,
) -> Option<(usize, ExecUnit)> {
    if hi < lo {
        return None;
    }
    for b in (lo..=hi).rev() {
        if schedule.bundle(b).is_free(ExecUnit::Alu0) {
            return Some((b, ExecUnit::Alu0));
        }
    }
    for b in (lo..=hi).rev() {
        if schedule.bundle(b).is_free(ExecUnit::Alu1) {
            return Some((b, ExecUnit::Alu1));
        }
    }
    None
}

/// Extend BB1 by one nop bundle, moving the loop instruction to the new last bundle.
fn push_loop_down(schedule: &mut Schedule, loop_idx: InstId) {
    let mut found = None;
    for (rel, b) in schedule.bb1.iter().enumerate() {
        for u in ExecUnit::ALL {
            if let Some(instr) = b.get(u) {
                if instr.id == loop_idx {
                    found = Some((rel, u));
                    break;
                }
            }
        }
        if found.is_some() {
            break;
        }
    }
    let (rel, unit) = found.expect("loop instruction missing from bb1");
    let loop_copy = schedule.bb1[rel].get(unit).cloned().unwrap();
    schedule.bb1[rel].clear(unit);

    let mut new_bundle = Bundle::new();
    new_bundle.set(ExecUnit::Branch, loop_copy);
    schedule.bb1.push(new_bundle);
    schedule.ii = schedule.bb1.len();
}

fn make_mov_reg(dst: u8, src: u8, id: InstId) -> Instruction {
    Instruction {
        id,
        opcode: Opcode::MovReg,
        pred: None,
        dest_reg: Some(dst),
        dest_pred: None,
        dest_special: None,
        src_regs: vec![src],
        imm: None,
        imm_bool: None,
        label: None,
    }
}
