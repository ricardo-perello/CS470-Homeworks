//! =========================================================================
//! STUDENT TODO — Rotating-register allocation for the pip schedule (PDF §3.3.2)
//! =========================================================================
//!
//! This file's `allocate()` currently delegates to `alloc_b` (the simple-loop
//! allocator), which is *wrong* for pip schedules: it ignores stage offsets,
//! doesn't use rotating registers, and won't close the interloop dependency
//! chain via the RRB. Replace the body.
//!
//! ## The four phases
//!
//! Let `S = #stages`, i.e. ⌈ len(BB1_raw) / II ⌉ stages in the pip schedule.
//!
//! **Phase 1 — Rotating regs for BB1 producers.**
//! Allocate fresh rotating registers to each BB1 instruction that writes an
//! x-register. Rotating regs start at x32 and step by `S + 1`:
//!     first producer → x32
//!     second producer → x32 + (S + 1)
//!     third producer → x32 + 2·(S + 1)
//!     ...
//! Walk BB1 producers in scheduling order (bundle 0 slot [Alu0, Alu1, Mult,
//! Mem, Branch] then bundle 1, …). The reason for stepping by `S + 1`:
//! a register's longest lifetime in a pipelined loop is `S` stages, so each
//! logical name needs `S + 1` physical slots to avoid overwrite.
//!
//! **Phase 2 — Non-rotating regs for loop invariants.**
//! Walk BB1 again in scheduling order; for each source operand classified as
//! `LoopInvariant(p)` (producer is in BB0), allocate the next free
//! non-rotating register to `p` (starting at x1). Record this binding — BB0
//! will use the same register for `p` in phase 4.
//!
//! **Phase 3 — Link BB1 operands to producers inside the loop.**
//! For each source operand of each BB1 instruction:
//!
//!   - **Local dep** `Local(p)` — the producer writes some rotating reg
//!     `x_S`. The consumer reads `x_D` where (eq. 3):
//!         x_D = x_S + (St(D) - St(S))
//!     St(·) = stage number of an instruction = ⌊ (bundle - bb1_start) / II ⌋.
//!
//!   - **Interloop dep** `Interloop { bb0, bb1 }` — same but +1 because the
//!     value crosses one loop back-edge (eq. 4):
//!         x_D = x_S + (St(D) - St(S)) + 1
//!
//!   - **Loop invariant** — use the non-rotating reg bound in phase 2.
//!
//! **Phase 4 — BB0 and BB2 allocations.**
//! Follow the four cases on PDF p. 12:
//!
//!   (a) BB0 producer of an interloop-consumed register r: use the *same*
//!       rotating register as the BB1 producer, but with stage offset
//!       `-St(P_bb1)` and iteration offset `+1` (written `x(Z + a + b)` in
//!       the PDF — here the `+1` accounts for BB0 "writing into the previous
//!       iteration" of the rotating window).
//!
//!   (b) BB0/BB2 producer with only a *local* dependency: allocate like
//!       alloc_b does (fresh non-rotating reg), unless already assigned in
//!       phase 1/2.
//!
//!   (c) BB2 post-loop dep from a BB1 producer: use the BB1 producer's
//!       rotating register offset by `-St(P)` (last iteration's value lives
//!       at stage `#stages - 1` when the loop ends).
//!
//!   (d) BB0/BB2 consumers of a loop invariant: read the non-rotating reg
//!       bound in phase 2.
//!
//! ## Helpful references
//!
//! - PDF §3.3.2, equations 3 and 4.
//! - PDF Figure 13 for a fully worked example (input → allocated).
//! - PDF Figure 14 for the lifetime-vs-#stages argument for `S + 1`.
//!
//! =========================================================================

use std::collections::HashMap;

use crate::alloc_b;
use crate::ir::*;
use crate::parser::Program;

pub fn allocate(
    schedule: &mut Schedule,
    program: &Program,
    deps: &DepTable,
    placement: &HashMap<InstId, Placement>,
) {
    // No loop.pip kernel: fall back to simple allocation so no-loop tests pass.
    if schedule.bb1.is_empty() || schedule.ii == 0 {
        alloc_b::allocate(schedule, program, deps, placement);
        return;
    }

    let ii = schedule.ii as i64;
    let bb1_start = schedule.bb0.len() as i64;
    let bb1_len = schedule.bb1.len() as i64;
    let stages = ((schedule.bb1.len() as i64) + ii - 1) / ii;
    let stride = stages + 1;

    // stage number for BB1 instructions (based on absolute bundle index).
    let stage_of = |id: InstId| -> i64 {
        let b = placement[&id].bundle as i64;
        (b - bb1_start) / ii
    };

    // ---- Phase 1: rotating regs for BB1 producers ----
    let mut rot_base: HashMap<InstId, i64> = HashMap::new();
    let mut k: i64 = 0;
    for rel in 0..schedule.bb1.len() {
        for unit in ExecUnit::ALL {
            if let Some(instr) = schedule.bb1[rel].get(unit) {
                if instr.dest_reg.is_some() {
                    rot_base.insert(instr.id, 32 + k * stride);
                    k += 1;
                }
            }
        }
    }
    for rel in 0..schedule.bb1.len() {
        for unit in ExecUnit::ALL {
            if let Some(instr) = schedule.bb1[rel].get_mut(unit) {
                if let Some(&rb) = rot_base.get(&instr.id) {
                    instr.dest_reg = Some(rb as u8);
                }
            }
        }
    }

    // ---- Phase 2: non-rotating regs for loop invariants ----
    let mut inv_reg: HashMap<InstId, u8> = HashMap::new();
    let mut next_nonrot: u8 = 1;
    for rel in 0..schedule.bb1.len() {
        for unit in ExecUnit::ALL {
            let Some(instr) = schedule.bb1[rel].get(unit) else { continue };
            if instr.id >= deps.len() { continue; }
            let id = instr.id;
            for dep in &deps[id].src_deps {
                if let Some(DepKind::LoopInvariant(p)) = dep {
                    inv_reg.entry(*p).or_insert_with(|| {
                        let r = next_nonrot;
                        next_nonrot += 1;
                        r
                    });
                }
            }
        }
    }

    // ---- Phase 4 prework: BB0 producers that feed interloop deps ----
    let mut bb0_interloop_reg: HashMap<InstId, u8> = HashMap::new();
    for inst_deps in deps {
        for dep in &inst_deps.src_deps {
            if let Some(DepKind::Interloop { bb0: Some(p0), bb1: p1 }) = dep {
                if let Some(&rb) = rot_base.get(p1) {
                    let st_p = stage_of(*p1);
                    let r = rb + (1 - st_p);
                    if r >= 0 && r <= 255 {
                        bb0_interloop_reg.insert(*p0, r as u8);
                    }
                }
            }
        }
    }

    // ---- Phase 4: assign remaining producer dest regs in BB0/BB2 ----
    let mut other_reg: HashMap<InstId, u8> = HashMap::new();
    for bidx in 0..schedule.total_bundles() {
        for unit in ExecUnit::ALL {
            let bundle = schedule.bundle_mut(bidx);
            let Some(instr) = bundle.get_mut(unit) else { continue };
            if instr.dest_reg.is_none() { continue; }

            if rot_base.contains_key(&instr.id) {
                continue; // BB1 already handled
            }
            if let Some(&r) = inv_reg.get(&instr.id) {
                instr.dest_reg = Some(r);
                continue;
            }
            if let Some(&r) = bb0_interloop_reg.get(&instr.id) {
                instr.dest_reg = Some(r);
                continue;
            }
            let r = other_reg.entry(instr.id).or_insert_with(|| {
                let rr = next_nonrot;
                next_nonrot += 1;
                rr
            });
            instr.dest_reg = Some(*r);
        }
    }

    // Helper to fetch a producer's assigned register (non-offset form).
    let producer_reg = |pid: InstId| -> Option<u8> {
        if let Some(&r) = inv_reg.get(&pid) {
            return Some(r);
        }
        if let Some(&r) = bb0_interloop_reg.get(&pid) {
            return Some(r);
        }
        if let Some(&r) = other_reg.get(&pid) {
            return Some(r);
        }
        if let Some(&rb) = rot_base.get(&pid) {
            return Some(rb as u8);
        }
        None
    };

    // ---- Phase 3: rewrite source operands (incl. rotating offsets) ----
    for bidx in 0..schedule.total_bundles() {
        for unit in ExecUnit::ALL {
            let bundle = schedule.bundle_mut(bidx);
            let Some(instr) = bundle.get_mut(unit) else { continue };
            if instr.id >= deps.len() { continue; }
            let cid = instr.id;

            let c_in_bb1 = (placement[&cid].bundle as i64) >= bb1_start
                && (placement[&cid].bundle as i64) < (bb1_start + bb1_len);
            let st_c = if c_in_bb1 { stage_of(cid) } else { 0 };

            for (op_i, dep) in deps[cid].src_deps.iter().enumerate() {
                if op_i >= instr.src_regs.len() {
                    continue;
                }
                match dep {
                    Some(DepKind::Local(p)) if c_in_bb1 && rot_base.contains_key(p) => {
                        let st_p = stage_of(*p);
                        let rb = rot_base[p];
                        instr.src_regs[op_i] = (rb + (st_c - st_p)) as u8;
                    }
                    Some(DepKind::Interloop { bb1: p, .. }) if c_in_bb1 && rot_base.contains_key(p) => {
                        let st_p = stage_of(*p);
                        let rb = rot_base[p];
                        instr.src_regs[op_i] = (rb + (st_c - st_p) + 1) as u8;
                    }
                    Some(DepKind::LoopInvariant(p)) => {
                        if let Some(&r) = inv_reg.get(p) {
                            instr.src_regs[op_i] = r;
                        }
                    }
                    Some(DepKind::PostLoop(p)) => {
                        if let Some(&rb) = rot_base.get(p) {
                            let st_p = stage_of(*p);
                            // Post-loop consumers read the last-iteration value, which lives
                            // at stage (#stages - 1) when the loop exits (PDF §3.3.2 (c)).
                            instr.src_regs[op_i] = (rb + ((stages - 1) - st_p)) as u8;
                        } else if let Some(r) = producer_reg(*p) {
                            instr.src_regs[op_i] = r;
                        }
                    }
                    Some(DepKind::Local(p)) | Some(DepKind::Interloop { bb0: Some(p), .. }) => {
                        if let Some(r) = producer_reg(*p) {
                            instr.src_regs[op_i] = r;
                        }
                    }
                    None => {
                        instr.src_regs[op_i] = next_nonrot;
                        next_nonrot += 1;
                    }
                    _ => {}
                }
            }
        }
    }

}
