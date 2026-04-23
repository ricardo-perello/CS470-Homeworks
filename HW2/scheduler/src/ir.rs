//! Intermediate representation for VLIW470 instructions, bundles, and schedules.
//!
//! Vocabulary follows the PDF directly:
//!   * An **instruction** is one assembly op (`add x1, x2, x3`, `ld x5, 0(x2)`, ...).
//!   * A **bundle** is 5 slots (ALU0, ALU1, Mult, Mem, Branch) executed in one cycle.
//!   * A **schedule** is an ordered sequence of bundles, split by basic block.

use serde::{Deserialize, Serialize};

pub type InstId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Opcode {
    Add,
    Addi,
    Sub,
    Mulu,
    Ld,
    St,
    Loop,
    LoopPip,
    Nop,
    MovReg,  // mov dst, src  (src is x-register)
    MovImm,  // mov dst, imm  (imm is integer)
    MovPred, // mov pX, true/false
    MovLC,   // mov LC, imm
    MovEC,   // mov EC, imm
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecUnit {
    Alu0,
    Alu1,
    Mult,
    Mem,
    Branch,
}

impl ExecUnit {
    pub fn idx(self) -> usize {
        match self {
            ExecUnit::Alu0 => 0,
            ExecUnit::Alu1 => 1,
            ExecUnit::Mult => 2,
            ExecUnit::Mem => 3,
            ExecUnit::Branch => 4,
        }
    }

    pub const ALL: [ExecUnit; 5] = [
        ExecUnit::Alu0,
        ExecUnit::Alu1,
        ExecUnit::Mult,
        ExecUnit::Mem,
        ExecUnit::Branch,
    ];
}

/// Coarse classification of which execution unit can run an opcode.
/// Two ALUs share the same class; the scheduler decides Alu0 vs Alu1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitClass {
    Alu,
    Mult,
    Mem,
    Branch,
}

impl UnitClass {
    pub fn of_opcode(op: Opcode) -> UnitClass {
        use Opcode::*;
        match op {
            Add | Addi | Sub | Nop | MovReg | MovImm | MovPred | MovLC | MovEC => UnitClass::Alu,
            Mulu => UnitClass::Mult,
            Ld | St => UnitClass::Mem,
            Loop | LoopPip => UnitClass::Branch,
        }
    }

    /// The execution units in this class, in preference order for packing.
    pub fn units(self) -> &'static [ExecUnit] {
        match self {
            UnitClass::Alu => &[ExecUnit::Alu0, ExecUnit::Alu1],
            UnitClass::Mult => &[ExecUnit::Mult],
            UnitClass::Mem => &[ExecUnit::Mem],
            UnitClass::Branch => &[ExecUnit::Branch],
        }
    }

    /// Number of parallel units in this class (for the II_res lower bound, eq. 1).
    pub fn num_units(self) -> usize {
        match self {
            UnitClass::Alu => 2,
            UnitClass::Mult => 1,
            UnitClass::Mem => 1,
            UnitClass::Branch => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialReg {
    LC,
    EC,
}

/// A single assembly instruction.
///
/// We keep original fields mutable so register-allocation passes can rewrite
/// `dest_reg` and `src_regs` in place when producing the final code.
#[derive(Debug, Clone)]
pub struct Instruction {
    /// Program-order index in the ORIGINAL input (stable across passes).
    pub id: InstId,
    pub opcode: Opcode,
    /// Predicate prefix `(pN)`, if any.
    pub pred: Option<u8>,

    /// X-register destination (None for st, loop, nop, mov pX/LC/EC).
    pub dest_reg: Option<u8>,
    /// Predicate destination (for mov pX, ...).
    pub dest_pred: Option<u8>,
    /// Special-register destination (LC/EC).
    pub dest_special: Option<SpecialReg>,

    /// X-register source operands in source-syntax order (not counting dst).
    /// E.g. `st x15, 0(x16)` → [15, 16]; `ld x13, 0(x14)` → [14].
    pub src_regs: Vec<u8>,

    /// Integer immediate (for addi, mov dst imm, ld/st offset, loop target).
    pub imm: Option<i64>,
    /// Bool immediate for `mov pX, true/false`.
    pub imm_bool: Option<bool>,
    /// Branch target as a bundle index (set at emit/schedule time, not by parser).
    pub label: Option<usize>,
}

impl Instruction {
    pub fn latency(&self) -> usize {
        match self.opcode {
            Opcode::Mulu => 3,
            _ => 1,
        }
    }

    pub fn unit_class(&self) -> UnitClass {
        UnitClass::of_opcode(self.opcode)
    }

    pub fn nop() -> Self {
        Instruction {
            id: usize::MAX,
            opcode: Opcode::Nop,
            pred: None,
            dest_reg: None,
            dest_pred: None,
            dest_special: None,
            src_regs: vec![],
            imm: None,
            imm_bool: None,
            label: None,
        }
    }
}

/// A single cycle worth of work: up to one instruction per execution unit.
#[derive(Debug, Clone, Default)]
pub struct Bundle {
    pub slots: [Option<Instruction>; 5],
}

impl Bundle {
    pub fn new() -> Self {
        Bundle {
            slots: [None, None, None, None, None],
        }
    }

    pub fn get(&self, u: ExecUnit) -> Option<&Instruction> {
        self.slots[u.idx()].as_ref()
    }

    pub fn get_mut(&mut self, u: ExecUnit) -> Option<&mut Instruction> {
        self.slots[u.idx()].as_mut()
    }

    pub fn set(&mut self, u: ExecUnit, i: Instruction) {
        self.slots[u.idx()] = Some(i);
    }

    pub fn clear(&mut self, u: ExecUnit) {
        self.slots[u.idx()] = None;
    }

    pub fn is_free(&self, u: ExecUnit) -> bool {
        self.slots[u.idx()].is_none()
    }
}

/// The final scheduled program, split into the three basic blocks.
#[derive(Debug, Clone)]
pub struct Schedule {
    pub bb0: Vec<Bundle>,
    pub bb1: Vec<Bundle>,
    pub bb2: Vec<Bundle>,
    /// Initiation Interval. For the simple-loop schedule this equals bb1.len().
    /// For the pip schedule it is the chosen II (possibly smaller than bb1.len()).
    pub ii: usize,
}

impl Schedule {
    pub fn empty() -> Self {
        Schedule {
            bb0: vec![],
            bb1: vec![],
            bb2: vec![],
            ii: 0,
        }
    }

    /// The bundle index at which BB1 begins in the flat output.
    pub fn bb1_start(&self) -> usize {
        self.bb0.len()
    }

    /// The bundle index at which BB2 begins.
    pub fn bb2_start(&self) -> usize {
        self.bb0.len() + self.bb1.len()
    }

    /// Total bundle count across BB0 + BB1 + BB2.
    pub fn total_bundles(&self) -> usize {
        self.bb0.len() + self.bb1.len() + self.bb2.len()
    }

    /// Iterate all bundles in output order.
    pub fn flat(&self) -> impl Iterator<Item = &Bundle> {
        self.bb0.iter().chain(self.bb1.iter()).chain(self.bb2.iter())
    }

    /// Access a bundle by flat index (BB0 bundles first, then BB1, then BB2).
    pub fn bundle(&self, idx: usize) -> &Bundle {
        if idx < self.bb0.len() {
            &self.bb0[idx]
        } else if idx < self.bb0.len() + self.bb1.len() {
            &self.bb1[idx - self.bb0.len()]
        } else {
            &self.bb2[idx - self.bb0.len() - self.bb1.len()]
        }
    }

    pub fn bundle_mut(&mut self, idx: usize) -> &mut Bundle {
        let bb0 = self.bb0.len();
        let bb1 = self.bb1.len();
        if idx < bb0 {
            &mut self.bb0[idx]
        } else if idx < bb0 + bb1 {
            &mut self.bb1[idx - bb0]
        } else {
            &mut self.bb2[idx - bb0 - bb1]
        }
    }
}

/// Dependency classification for a single source operand.
#[derive(Debug, Clone)]
pub enum DepKind {
    /// Producer and consumer in the same basic block, producer before consumer.
    Local(InstId),
    /// Consumer in BB1, producer is a BB1 writer whose reaching definition
    /// comes from the *previous* iteration. May have a BB0 initializer too.
    Interloop { bb0: Option<InstId>, bb1: InstId },
    /// Producer in BB0, no BB1 producer of the same register.
    LoopInvariant(InstId),
    /// Consumer in BB2, producer in BB1 (last iteration's value).
    PostLoop(InstId),
}

/// Per-instruction dependency info: one entry per source operand, in the
/// same order as `Instruction.src_regs`.
#[derive(Debug, Clone, Default)]
pub struct InstrDeps {
    pub src_deps: Vec<Option<DepKind>>,
}

pub type DepTable = Vec<InstrDeps>;

/// Placement of a scheduled instruction.
#[derive(Debug, Clone, Copy)]
pub struct Placement {
    /// Absolute bundle index in the flat output (0-based, counting BB0+BB1+BB2).
    pub bundle: usize,
    pub unit: ExecUnit,
}
