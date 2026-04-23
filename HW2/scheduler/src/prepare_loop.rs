//! =========================================================================
//! STUDENT TODO — Loop prologue & stage predication (PDF §3.4)
//! =========================================================================
//!
//! After `alloc_r` has renamed registers, the pip schedule still has
//! `#stages × II` bundles of body code. The `loop.pip` hardware expects a
//! single II-bundle kernel, with each instruction predicated on its stage's
//! predicate. This pass does that collapse + predication.
//!
//! ## Steps
//!
//! 1. **Predicate every BB1 instruction** with `p(32 + stage_of_instruction)`:
//!        stage = (bundle - bb1_start) / II
//!    (Exception: the `loop.pip` instruction itself is NOT predicated.)
//!
//! 2. **Collapse BB1** from `#stages × II` bundles down to II bundles. For
//!    each instruction at bundle `b` on unit `u`, move it to bundle
//!    `bb1_start + (b - bb1_start) mod II`. Multiple stages overlap in the
//!    same II-bundle kernel — they run concurrently (the `loop.pip` hardware
//!    uses the RRB + rotating predicates to enable/disable them per cycle).
//!    NOTE: this relies on modulo slot reservation in `schedule_pip` having
//!    guaranteed no two stages claim the same `(bundle mod II, unit)`.
//!
//! 3. **Prologue setup in BB0** — insert two ALU ops into the bundle right
//!    before the `loop.pip` instruction:
//!        mov p32, true            // enable stage 0
//!        mov EC, (#stages - 1)    // set epilog counter
//!    If that bundle already has both Alu0 and Alu1 full, insert a *new*
//!    bundle right before the loop with the movs (shifting everything after
//!    by +1 and updating the loop.pip target accordingly).
//!
//! ## Helpful references
//!
//! - PDF Fig. 5 — precise semantics of `loop.pip`.
//! - PDF Fig. 15 — final code for the handout example.
//! - PDF §3.4 — the preparation walkthrough.
//!
//! =========================================================================

use crate::ir::*;
use crate::parser::Program;

pub fn prepare(schedule: &mut Schedule, _program: &Program) {
    if schedule.bb1.is_empty() || schedule.ii == 0 {
        return;
    }

    let ii = schedule.ii;
    let stages = (schedule.bb1.len() + ii - 1) / ii;

    // 1) Predicate every BB1 instruction with p(32 + stage), except loop.pip itself.
    for (rel, b) in schedule.bb1.iter_mut().enumerate() {
        let stage = rel / ii;
        let pred = 32u8.saturating_add(stage as u8);
        for u in ExecUnit::ALL {
            if let Some(i) = b.get_mut(u) {
                if matches!(i.opcode, Opcode::LoopPip) {
                    continue;
                }
                i.pred = Some(pred);
            }
        }
    }

    // 2) Collapse BB1 from (stages * II) bundles down to II bundles.
    let old_bb1 = std::mem::take(&mut schedule.bb1);
    let mut kernel: Vec<Bundle> = (0..ii).map(|_| Bundle::new()).collect();
    for (rel, b) in old_bb1.into_iter().enumerate() {
        for (slot_idx, maybe_i) in b.slots.into_iter().enumerate() {
            if let Some(i) = maybe_i {
                let u = ExecUnit::ALL[slot_idx];
                let dst = if matches!(i.opcode, Opcode::LoopPip) {
                    ii - 1
                } else {
                    rel % ii
                };
                kernel[dst].set(u, i);
            }
        }
    }
    schedule.bb1 = kernel;

    // 3) Prologue setup in BB0: mov p32,true and mov EC,(stages-1).
    let ec_imm = (stages as i64).saturating_sub(1);
    let mov_p32 = Instruction {
        id: usize::MAX - 1,
        opcode: Opcode::MovPred,
        pred: None,
        dest_reg: None,
        dest_pred: Some(32),
        dest_special: None,
        src_regs: vec![],
        imm: None,
        imm_bool: Some(true),
        label: None,
    };
    let mov_ec = Instruction {
        id: usize::MAX - 2,
        opcode: Opcode::MovEC,
        pred: None,
        dest_reg: None,
        dest_pred: None,
        dest_special: Some(SpecialReg::EC),
        src_regs: vec![],
        imm: Some(ec_imm),
        imm_bool: None,
        label: None,
    };

    if schedule.bb0.is_empty() {
        schedule.bb0.push(Bundle::new());
    }
    let mut insert_new = false;
    let mut ec_already_placed = false;
    {
        let last = schedule.bb0.last_mut().unwrap();
        let alu0_free = last.is_free(ExecUnit::Alu0);
        let alu1_free = last.is_free(ExecUnit::Alu1);
        if alu0_free && alu1_free {
            // Match reference ordering: EC in Alu0, p32 in Alu1.
            last.set(ExecUnit::Alu0, mov_ec.clone());
            last.set(ExecUnit::Alu1, mov_p32.clone());
            ec_already_placed = true;
        } else if alu0_free ^ alu1_free {
            // One ALU free: put EC here, put p32 in a new bundle (matches many refs).
            if alu0_free {
                last.set(ExecUnit::Alu0, mov_ec.clone());
            } else {
                last.set(ExecUnit::Alu1, mov_ec.clone());
            }
            ec_already_placed = true;
            insert_new = true;
        } else {
            insert_new = true;
        }
    }
    if insert_new {
        let mut b = Bundle::new();
        b.set(ExecUnit::Alu0, mov_p32);
        if !ec_already_placed && b.is_free(ExecUnit::Alu1) {
            b.set(ExecUnit::Alu1, mov_ec);
        }
        schedule.bb0.push(b);
    }

    // If BB0 grew, BB1 start shifted; retarget loop.pip label.
    let new_bb1_start = schedule.bb0.len();
    for b in schedule.bb1.iter_mut() {
        if let Some(lp) = b.get_mut(ExecUnit::Branch) {
            if matches!(lp.opcode, Opcode::LoopPip) {
                lp.label = Some(new_bb1_start);
            }
        }
    }
}
