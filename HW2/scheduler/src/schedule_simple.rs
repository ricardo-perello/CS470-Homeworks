//! ASAP scheduling for the **simple-loop** output (the `loop` instruction).
//!
//! Algorithm (PDF §3.2.1):
//!
//! 1. Walk instructions in program order, splitting by basic block.
//! 2. For each instruction, compute `min_cycle` = max over its deps of
//!    `producer.bundle + latency(producer)`. This covers local, loop-invariant,
//!    and post-loop deps uniformly (since post-loop reduces to `c >= p + λ` in
//!    absolute bundle coords after the N-iteration unrolling cancels out).
//! 3. Place the instruction at the earliest bundle ≥ `min_cycle` with a free
//!    execution unit of the right class. ALUs prefer Alu0 then Alu1.
//! 4. For BB1, schedule all instructions *except* the loop in program order,
//!    then place the loop at the last bundle used (in the Branch slot).
//! 5. After BB1 is scheduled, verify each interloop dep satisfies
//!    `S(P) + λ(P) ≤ S(C) + II` (eq. 2). If any is violated, extend BB1 by
//!    inserting a nop bundle before the loop and retry.
//! 6. If a BB0 producer has latency that extends past the raw BB0 end, pad BB0
//!    with trailing nops (PDF §3.2 — bubbles go outside the loop body).

use std::collections::HashMap;

use crate::deps;
use crate::ir::*;
use crate::parser::Program;

pub fn schedule(program: &Program, dep_table: &DepTable) -> (Schedule, HashMap<InstId, Placement>) {
    let _ = deps::analyze; // keep module referenced
    let mut placement: HashMap<InstId, Placement> = HashMap::new();

    // --- BB0: straight-line ASAP ---
    let mut bb0 = schedule_straight(program, dep_table, program.bb0_range(), 0, &mut placement);
    pad_for_latencies(&mut bb0, program, &placement, 0);

    // --- BB1: ASAP for body, loop at end, then interloop retry ---
    let bb0_len = bb0.len();
    let bb1 = if program.bb1_range().is_empty() {
        vec![]
    } else {
        schedule_bb1(program, dep_table, bb0_len, &mut placement)
    };
    let bb1_len = bb1.len();

    // --- BB2: straight-line ASAP starting at bb0_len + bb1_len ---
    let bb2 = schedule_straight(
        program,
        dep_table,
        program.bb2_range(),
        bb0_len + bb1_len,
        &mut placement,
    );

    let ii = bb1_len;
    let sched = Schedule { bb0, bb1, bb2, ii };
    (sched, placement)
}

/// Straight-line ASAP schedule for a BB (used for BB0 and BB2 — no loop handling).
fn schedule_straight(
    program: &Program,
    dep_table: &DepTable,
    range: std::ops::Range<usize>,
    bb_start: usize,
    placement: &mut HashMap<InstId, Placement>,
) -> Vec<Bundle> {
    let mut bundles: Vec<Bundle> = vec![];
    for idx in range {
        let instr = &program.instrs[idx];
        let min_abs = compute_min_cycle(&dep_table[idx], placement, program, bb_start);
        let min_rel = min_abs.saturating_sub(bb_start);
        let (rel, unit) = find_slot(&mut bundles, min_rel, instr.unit_class());
        bundles[rel].set(unit, instr.clone());
        placement.insert(
            idx,
            Placement {
                bundle: bb_start + rel,
                unit,
            },
        );
    }
    bundles
}

/// Schedule BB1 for the simple-loop case.
/// - Body (all BB1 instructions except the loop) in program order, ASAP.
/// - Loop placed in the Branch slot of the last used bundle.
/// - Interloop deps verified via eq. (2); BB1 extended (loop pushed down) if violated.
fn schedule_bb1(
    program: &Program,
    dep_table: &DepTable,
    bb_start: usize,
    placement: &mut HashMap<InstId, Placement>,
) -> Vec<Bundle> {
    let loop_idx = program.loop_idx().expect("bb1 nonempty => loop exists");
    let mut bundles: Vec<Bundle> = vec![];

    // Schedule the body (program order, skip loop).
    for idx in program.bb1_range() {
        if idx == loop_idx {
            continue;
        }
        let instr = &program.instrs[idx];
        let min_abs = compute_min_cycle(&dep_table[idx], placement, program, bb_start);
        let min_rel = min_abs.saturating_sub(bb_start);
        let (rel, unit) = find_slot(&mut bundles, min_rel, instr.unit_class());
        bundles[rel].set(unit, instr.clone());
        placement.insert(
            idx,
            Placement {
                bundle: bb_start + rel,
                unit,
            },
        );
    }

    // Place the loop instruction at the last used bundle (Branch slot).
    let last_rel = if bundles.is_empty() { 0 } else { bundles.len() - 1 };
    let (loop_rel, loop_unit) = find_slot(&mut bundles, last_rel, UnitClass::Branch);
    let mut loop_instr = program.instrs[loop_idx].clone();
    loop_instr.label = Some(bb_start); // bundle index of BB1 start in the output
    bundles[loop_rel].set(loop_unit, loop_instr);
    placement.insert(
        loop_idx,
        Placement {
            bundle: bb_start + loop_rel,
            unit: loop_unit,
        },
    );

    // Verify eq. (2) for each interloop dep; extend BB1 (push loop down) until satisfied.
    loop {
        let ii = bundles.len();
        let mut violated = false;
        'outer: for idx in program.bb1_range() {
            if idx == loop_idx {
                continue;
            }
            for dep in &dep_table[idx].src_deps {
                if let Some(DepKind::Interloop { bb1, .. }) = dep {
                    let p_cycle = placement[bb1].bundle;
                    let p_lat = program.instrs[*bb1].latency();
                    let c_cycle = placement[&idx].bundle;
                    // S(P) + λ(P) ≤ S(C) + II
                    if p_cycle + p_lat > c_cycle + ii {
                        violated = true;
                        break 'outer;
                    }
                }
            }
        }
        if !violated {
            break;
        }
        push_loop_down(&mut bundles, placement, loop_idx, bb_start, &program.instrs[loop_idx]);
    }

    bundles
}

/// Insert a nop bundle before the current loop bundle (growing BB1 by 1).
fn push_loop_down(
    bundles: &mut Vec<Bundle>,
    placement: &mut HashMap<InstId, Placement>,
    loop_idx: InstId,
    bb_start: usize,
    loop_template: &Instruction,
) {
    // Remove loop from its current slot.
    let pl = placement[&loop_idx];
    let old_rel = pl.bundle - bb_start;
    bundles[old_rel].clear(pl.unit);
    // Append a fresh bundle and put the loop in its Branch slot.
    bundles.push(Bundle::new());
    let new_rel = bundles.len() - 1;
    let mut copy = loop_template.clone();
    copy.label = Some(bb_start);
    bundles[new_rel].set(ExecUnit::Branch, copy);
    placement.insert(
        loop_idx,
        Placement {
            bundle: bb_start + new_rel,
            unit: ExecUnit::Branch,
        },
    );
}

/// Minimum absolute cycle at which an instruction can be placed, based on its
/// local/invariant/post-loop deps. Interloop deps are intentionally NOT
/// considered here — they're verified post-hoc via eq. (2).
fn compute_min_cycle(
    inst_deps: &InstrDeps,
    placement: &HashMap<InstId, Placement>,
    program: &Program,
    bb_start: usize,
) -> usize {
    let mut min_cycle = bb_start;
    for dep in &inst_deps.src_deps {
        match dep {
            Some(DepKind::Local(p))
            | Some(DepKind::LoopInvariant(p))
            | Some(DepKind::PostLoop(p)) => {
                let p_bundle = placement[p].bundle;
                let p_lat = program.instrs[*p].latency();
                min_cycle = min_cycle.max(p_bundle + p_lat);
            }
            // Interloop: handled post-hoc.
            Some(DepKind::Interloop { .. }) | None => {}
        }
    }
    min_cycle
}

/// Return the earliest (rel_cycle, unit) at/after `min_rel` with a free slot
/// of the requested class. Grows `bundles` with nops as needed.
fn find_slot(
    bundles: &mut Vec<Bundle>,
    min_rel: usize,
    class: UnitClass,
) -> (usize, ExecUnit) {
    let mut rel = min_rel;
    loop {
        while bundles.len() <= rel {
            bundles.push(Bundle::new());
        }
        for &unit in class.units() {
            if bundles[rel].is_free(unit) {
                return (rel, unit);
            }
        }
        rel += 1;
    }
}

/// Extend a BB's bundle list with nop bundles at the end so that every
/// producer placed inside finishes strictly before the BB ends. This matches
/// the PDF's "bubbles go outside the loop body" rule — for BB0 we pad after
/// scheduling its instructions so that BB1 reads see ready values on entry.
fn pad_for_latencies(
    bundles: &mut Vec<Bundle>,
    program: &Program,
    placement: &HashMap<InstId, Placement>,
    bb_start: usize,
) {
    let mut required = bundles.len();
    for (idx, pl) in placement {
        if pl.bundle < bb_start {
            continue;
        }
        let rel = pl.bundle - bb_start;
        if rel >= bundles.len() {
            continue;
        }
        let lat = program.instrs[*idx].latency();
        required = required.max(rel + lat);
    }
    while bundles.len() < required {
        bundles.push(Bundle::new());
    }
}
