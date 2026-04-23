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
use crate::schedule_simple;

pub fn schedule(
    program: &Program,
    dep_table: &DepTable,
) -> (Schedule, HashMap<InstId, Placement>) {
    // FIXME(student): replace this body with a real modulo scheduler.
    let (mut s, placement) = schedule_simple::schedule(program, dep_table);

    // Tag the branch slot with the `loop.pip` opcode so the emitted JSON at
    // least reads as a pip schedule. Everything else still needs rewriting
    // (stage predicates, rotating regs, kernel folding).
    for bundle in s.bb1.iter_mut() {
        if let Some(instr) = bundle.get_mut(ExecUnit::Branch) {
            if matches!(instr.opcode, Opcode::Loop) {
                instr.opcode = Opcode::LoopPip;
            }
        }
    }

    (s, placement)
}
