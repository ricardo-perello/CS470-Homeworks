//! Dependency analysis — produces the equivalent of Table 2 in the PDF.
//!
//! For each instruction, for each source operand, we classify the reaching
//! producer(s). Categories (PDF §3.2):
//!   * **Local**: same BB, producer before consumer in program order.
//!   * **Interloop**: consumer in BB1, and BB1 has a producer of this register
//!     whose reaching definition wraps around the loop back-edge. Pairs with
//!     the BB0 initializer (if any) — both go into the same `Interloop` entry.
//!   * **LoopInvariant**: producer in BB0, no BB1 producer for this register.
//!   * **PostLoop**: consumer in BB2, producer in BB1.
//!
//! Operands that have no producer anywhere get `None` and are handled by
//! alloc_b's "unused register" phase 4.

use std::collections::HashMap;

use crate::ir::*;
use crate::parser::Program;

pub fn analyze(program: &Program) -> DepTable {
    // Latest BB0 producer per register (BB0 is straight-line, so "latest" = the
    // last write to that register in BB0).
    let mut bb0_prod: HashMap<u8, InstId> = HashMap::new();
    for idx in program.bb0_range() {
        if let Some(r) = program.instrs[idx].dest_reg {
            bb0_prod.insert(r, idx);
        }
    }

    // Latest BB1 producer per register. The PDF guarantees at most one BB1
    // producer per register (we keep the last one we see, which matches that).
    let mut bb1_prod: HashMap<u8, InstId> = HashMap::new();
    for idx in program.bb1_range() {
        if let Some(r) = program.instrs[idx].dest_reg {
            bb1_prod.insert(r, idx);
        }
    }

    let mut table = Vec::with_capacity(program.instrs.len());
    for (idx, instr) in program.instrs.iter().enumerate() {
        let src_deps = instr
            .src_regs
            .iter()
            .map(|&r| classify(r, idx, program, &bb0_prod, &bb1_prod))
            .collect();
        table.push(InstrDeps { src_deps });
    }
    table
}

fn classify(
    r: u8,
    consumer: InstId,
    program: &Program,
    bb0_prod: &HashMap<u8, InstId>,
    bb1_prod: &HashMap<u8, InstId>,
) -> Option<DepKind> {
    if program.in_bb0(consumer) {
        // Only local deps make sense in BB0 (no backward-flowing info).
        if let Some(p) = latest_producer_in_range(r, program, 0..consumer) {
            return Some(DepKind::Local(p));
        }
        None
    } else if program.in_bb1(consumer) {
        // Local BB1 producer comes first (same iteration, earlier in program order).
        if let Some(p) =
            latest_producer_in_range(r, program, program.bb1_range().start..consumer)
        {
            return Some(DepKind::Local(p));
        }
        // Otherwise: look for a BB1 producer at-or-after the consumer. Its
        // value flows through the loop back-edge, so it's an interloop dep.
        let bb1_after = first_producer_in_range(r, program, consumer..program.bb1_range().end);
        let bb0_p = bb0_prod.get(&r).copied();
        match (bb1_after, bb0_p) {
            (Some(b1), _) => Some(DepKind::Interloop {
                bb0: bb0_p,
                bb1: b1,
            }),
            (None, Some(b0)) => Some(DepKind::LoopInvariant(b0)),
            (None, None) => None,
        }
    } else {
        // BB2: local producer in BB2 first, else post-loop from BB1, else loop-invariant from BB0.
        if let Some(p) =
            latest_producer_in_range(r, program, program.bb2_range().start..consumer)
        {
            return Some(DepKind::Local(p));
        }
        if let Some(&p) = bb1_prod.get(&r) {
            return Some(DepKind::PostLoop(p));
        }
        if let Some(&p) = bb0_prod.get(&r) {
            return Some(DepKind::LoopInvariant(p));
        }
        None
    }
}

fn latest_producer_in_range(
    r: u8,
    program: &Program,
    range: std::ops::Range<usize>,
) -> Option<InstId> {
    for idx in range.rev() {
        if program.instrs[idx].dest_reg == Some(r) {
            return Some(idx);
        }
    }
    None
}

fn first_producer_in_range(
    r: u8,
    program: &Program,
    range: std::ops::Range<usize>,
) -> Option<InstId> {
    for idx in range {
        if program.instrs[idx].dest_reg == Some(r) {
            return Some(idx);
        }
    }
    None
}
