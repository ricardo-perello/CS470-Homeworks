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
    // FIXME(student): replace this body with rotating-register allocation.
    alloc_b::allocate(schedule, program, deps, placement);
}
