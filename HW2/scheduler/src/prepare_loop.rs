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

pub fn prepare(schedule: &mut Schedule, program: &Program) {
    // FIXME(student): implement predication + kernel folding + prologue.
    let _ = schedule;
    let _ = program;
}
