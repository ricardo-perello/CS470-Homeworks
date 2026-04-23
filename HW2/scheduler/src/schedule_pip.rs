//! =========================================================================
//! STUDENT TODO — Modulo scheduling for the `loop.pip` instruction (PDF §3.2.2)
//! =========================================================================
//!
//! This file's `schedule()` currently falls back to the simple-loop scheduler
//! (with the `loop` opcode rewritten to `loop.pip`). That is WRONG for any
//! test with a loop that needs software pipelining — your job is to replace
//! the body with a real modulo scheduler.
//!
//! ## What you need to build
//!
//! 1. **Initiation Interval lower bound** (PDF eq. 1):
//!    `II_res = max_i( ⌈ N_i / U_i ⌉ )`
//!    where `N_i` is the count of BB1 instructions of class `i` (Alu / Mult /
//!    Mem / Branch) and `U_i` is the number of units of that class (2 for
//!    ALU, 1 otherwise). Helper: `UnitClass::num_units()`.
//!
//! 2. **II search loop** — starting at II = II_res, attempt to schedule the
//!    loop body under the current II. If any check fails (cannot fit an
//!    instruction; interloop eq. 2 violated), **bump II by 1 and retry**.
//!    Scheduling succeeds when every BB1 instruction is placed AND every
//!    interloop dep satisfies `S(P) + λ(P) ≤ S(C) + II`.
//!
//! 3. **Modulo slot reservation** — BB1 expands into `#stages × II` bundles
//!    where `#stages = ⌈len(BB1) / II⌉`. When you place instruction X at
//!    bundle `b` on unit `u`, mark every bundle `b + k·II` (for all stages
//!    k) as also "using" that unit — no other instruction may claim it.
//!    Concretely: maintain a resource table keyed by `(b mod II, unit)` that
//!    marks reserved slots. See PDF Figure 11.
//!
//! 4. **BB0 and BB2** scheduling is the same as the simple case (straight-
//!    line ASAP). You can reuse the helpers in `schedule_simple.rs` — pull
//!    them out into a shared module if needed.
//!
//! 5. **Loop instruction** lives in the last bundle of BB1 (Branch slot) with
//!    opcode `LoopPip` and its `label` set to the bundle index where BB1
//!    begins in the output.
//!
//! ## Helpful references
//!
//! - `crate::ir::UnitClass::num_units()` — denominators for eq. 1.
//! - `crate::ir::DepKind::Interloop` — what to check for eq. 2.
//! - PDF Fig. 11 for an II=2 → II=3 example worked end to end.
//! - The visualizer (`HW2/simulator/visualize.html`) is great for comparing
//!   your `pip.json` against the reference when a test fails.
//!
//! =========================================================================

use std::collections::HashMap;

use crate::ir::*;
use crate::parser::Program;

pub fn schedule(
    program: &Program,
    dep_table: &DepTable,
) -> (Schedule, HashMap<InstId, Placement>) {
    let mut placement: HashMap<InstId, Placement> = HashMap::new();

    // --- BB0: straight-line ASAP (same as simple) ---
    let mut bb0 = schedule_straight(program, dep_table, program.bb0_range(), 0, &mut placement);
    pad_for_latencies(&mut bb0, program, &placement, 0);

    let bb0_len = bb0.len();

    // --- BB1: modulo schedule loop body (software pipelining) ---
    let (bb1, ii) = if program.bb1_range().is_empty() {
        (vec![], 0)
    } else {
        schedule_bb1_modulo(program, dep_table, bb0_len, &mut placement)
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

    (Schedule { bb0, bb1, bb2, ii }, placement)
}

// ---- helpers (mostly adapted from schedule_simple.rs) ----

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
        let (rel, unit) = find_slot_linear(&mut bundles, min_rel, instr.unit_class());
        bundles[rel].set(unit, instr.clone());
        placement.insert(idx, Placement { bundle: bb_start + rel, unit });
    }
    bundles
}

fn schedule_bb1_modulo(
    program: &Program,
    dep_table: &DepTable,
    bb_start: usize,
    placement: &mut HashMap<InstId, Placement>,
) -> (Vec<Bundle>, usize) {
    let loop_idx = program.loop_idx().expect("bb1 nonempty => loop exists");

    // Compute II_res (eq. 1) over BB1 body (excluding the loop instruction).
    let mut n_alu = 0usize;
    let mut n_mult = 0usize;
    let mut n_mem = 0usize;
    let mut n_branch = 0usize;
    for idx in program.bb1_range() {
        if idx == loop_idx {
            continue;
        }
        match program.instrs[idx].unit_class() {
            UnitClass::Alu => n_alu += 1,
            UnitClass::Mult => n_mult += 1,
            UnitClass::Mem => n_mem += 1,
            UnitClass::Branch => n_branch += 1,
        }
    }
    let mut ii_res = 1;
    ii_res = ii_res.max((n_alu + UnitClass::Alu.num_units() - 1) / UnitClass::Alu.num_units());
    ii_res = ii_res.max((n_mult + UnitClass::Mult.num_units() - 1) / UnitClass::Mult.num_units());
    ii_res = ii_res.max((n_mem + UnitClass::Mem.num_units() - 1) / UnitClass::Mem.num_units());
    ii_res = ii_res.max((n_branch + UnitClass::Branch.num_units() - 1) / UnitClass::Branch.num_units());

    let mut ii = ii_res;
    loop {
        if let Some((bb1, ok)) = try_schedule_modulo(program, dep_table, bb_start, placement, loop_idx, ii) {
            if ok {
                return (bb1, ii);
            }
        }
        ii += 1;
    }
}

fn try_schedule_modulo(
    program: &Program,
    dep_table: &DepTable,
    bb_start: usize,
    placement: &mut HashMap<InstId, Placement>,
    loop_idx: InstId,
    ii: usize,
) -> Option<(Vec<Bundle>, bool)> {
    // resource table: (cycle_mod_ii, unit) -> occupied
    let mut used: HashMap<(usize, ExecUnit), bool> = HashMap::new();
    let mut bundles: Vec<Bundle> = vec![];

    // Place BB1 body instructions in program order, modulo-reserving by (c mod II, unit).
    for idx in program.bb1_range() {
        if idx == loop_idx {
            continue;
        }
        let instr = &program.instrs[idx];
        let min_abs = compute_min_cycle(&dep_table[idx], placement, program, bb_start);
        let mut abs = min_abs.max(bb_start);
        loop {
            let rel = abs - bb_start;
            while bundles.len() <= rel {
                bundles.push(Bundle::new());
            }
            let slot = abs % ii;
            let mut placed = None;
            for &unit in instr.unit_class().units() {
                if bundles[rel].is_free(unit) && !used.get(&(slot, unit)).copied().unwrap_or(false) {
                    placed = Some(unit);
                    break;
                }
            }
            if let Some(unit) = placed {
                bundles[rel].set(unit, instr.clone());
                used.insert((slot, unit), true);
                placement.insert(idx, Placement { bundle: abs, unit });
                break;
            }
            abs += 1;
        }
    }

    // Ensure we have at least one bundle to host the loop.pip instruction.
    if bundles.is_empty() {
        bundles.push(Bundle::new());
    }
    let last_rel = bundles.len() - 1;

    // Place loop.pip at the end in the Branch slot, growing if needed.
    let mut loop_abs = bb_start + last_rel;
    loop {
        let rel = loop_abs - bb_start;
        while bundles.len() <= rel {
            bundles.push(Bundle::new());
        }
        let slot = loop_abs % ii;
        if bundles[rel].is_free(ExecUnit::Branch) && !used.get(&(slot, ExecUnit::Branch)).copied().unwrap_or(false) {
            let mut loop_i = program.instrs[loop_idx].clone();
            loop_i.opcode = Opcode::LoopPip;
            loop_i.label = Some(bb_start);
            bundles[rel].set(ExecUnit::Branch, loop_i);
            used.insert((slot, ExecUnit::Branch), true);
            placement.insert(loop_idx, Placement { bundle: loop_abs, unit: ExecUnit::Branch });
            break;
        }
        loop_abs += 1;
    }

    // Verify eq. (2) for interloop deps under this II.
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
                if p_cycle + p_lat > c_cycle + ii {
                    violated = true;
                    break 'outer;
                }
            }
        }
    }

    Some((bundles, !violated))
}

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
            Some(DepKind::Interloop { .. }) | None => {}
        }
    }
    min_cycle
}

fn find_slot_linear(bundles: &mut Vec<Bundle>, min_rel: usize, class: UnitClass) -> (usize, ExecUnit) {
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
