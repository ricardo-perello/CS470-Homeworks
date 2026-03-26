use std::collections::VecDeque;

use serde::Serialize;

use crate::pipeline::propagate_one_cycle;
use crate::types::{ActiveListEntry, Forwarding, IntegerQueueEntry};

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    #[serde(rename = "ActiveList")]
    pub active_list: Vec<ActiveListEntry>,
    #[serde(rename = "BusyBitTable")]
    pub busy_bit_table: Vec<bool>,
    #[serde(rename = "DecodedPCs")]
    pub decoded_pcs: Vec<u64>,
    #[serde(rename = "Exception")]
    pub exception: bool,
    #[serde(rename = "ExceptionPC")]
    pub exception_pc: u64,
    #[serde(rename = "FreeList")]
    pub free_list: Vec<usize>,
    #[serde(rename = "IntegerQueue")]
    pub integer_queue: Vec<IntegerQueueEntry>,
    #[serde(rename = "PC")]
    pub pc: u64,
    #[serde(rename = "PhysicalRegisterFile")]
    pub physical_register_file: Vec<u64>,
    #[serde(rename = "RegisterMapTable")]
    pub register_map_table: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct ProcessorState {
    pub pc: u64,
    pub physical_register_file: [u64; 64],
    pub decoded_pcs: Vec<u64>,
    pub exception: bool,
    pub exception_pc: u64,
    pub register_map_table: [usize; 32],
    pub free_list: VecDeque<usize>,
    pub busy_bit_table: [bool; 64],
    pub active_list: VecDeque<ActiveListEntry>,
    pub integer_queue: Vec<IntegerQueueEntry>,

    // Internal, not logged
    pub decoded_instructions: Vec<crate::types::Instruction>,
    pub alu_pipeline: [[Option<crate::types::AluSlot>; 2]; 4],
    pub exception_will_clear: bool,
}

impl ProcessorState {
    pub fn new() -> Self {
        let mut rmt = [0usize; 32];
        for i in 0..32 {
            rmt[i] = i;
        }
        let mut free_list = VecDeque::new();
        for p in 32..64 {
            free_list.push_back(p);
        }

        Self {
            pc: 0,
            physical_register_file: [0u64; 64],
            decoded_pcs: Vec::new(),
            exception: false,
            exception_pc: 0,
            register_map_table: rmt,
            free_list,
            busy_bit_table: [false; 64],
            active_list: VecDeque::new(),
            integer_queue: Vec::new(),
            decoded_instructions: Vec::new(),
            alu_pipeline: [[None; 2]; 4],
            exception_will_clear: false,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            active_list: self.active_list.iter().cloned().collect(),
            busy_bit_table: self.busy_bit_table.iter().copied().collect(),
            decoded_pcs: self.decoded_pcs.clone(),
            exception: self.exception,
            exception_pc: self.exception_pc,
            free_list: self.free_list.iter().copied().collect(),
            integer_queue: self.integer_queue.clone(),
            pc: self.pc,
            physical_register_file: self.physical_register_file.iter().copied().collect(),
            register_map_table: self.register_map_table.iter().copied().collect(),
        }
    }

    pub fn has_work(&self, program_len: u64) -> bool {
        if self.exception || self.exception_will_clear {
            return true;
        }
        if self.pc < program_len {
            return true;
        }
        if !self.decoded_pcs.is_empty() {
            return true;
        }
        if !self.decoded_instructions.is_empty() {
            return true;
        }
        if !self.integer_queue.is_empty() {
            return true;
        }
        if !self.active_list.is_empty() {
            return true;
        }
        for a in 0..4 {
            for s in 0..2 {
                if self.alu_pipeline[a][s].is_some() {
                    return true;
                }
            }
        }
        false
    }

    pub fn propagate(&mut self, program: &[crate::types::Instruction]) {
        propagate_one_cycle(self, program);
    }

    pub fn current_forwarding(&self) -> Vec<Forwarding> {
        let mut out = Vec::new();
        for alu in 0..4 {
            if let Some(slot) = self.alu_pipeline[alu][1] {
                out.push(Forwarding {
                    pc: slot.pc,
                    dest_phys: slot.dest_phys,
                    value: slot.value,
                    exception: slot.exception,
                });
            }
        }
        out
    }
}

