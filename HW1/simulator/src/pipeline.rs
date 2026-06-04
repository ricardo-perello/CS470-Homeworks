use std::collections::HashMap;

use crate::state::ProcessorState;
use crate::types::{ActiveListEntry, AluSlot, Forwarding, Instruction, IntegerQueueEntry, OpCode, OperandB};

fn signext_imm_to_u64(x: i64) -> u64 {
    x as u64
}

fn compute_alu_result(op: OpCode, a: u64, b: u64) -> (u64, bool) {
    match op {
        OpCode::Add => (a.wrapping_add(b), false),
        OpCode::Sub => (a.wrapping_sub(b), false),
        OpCode::Mulu => (a.wrapping_mul(b), false),
        OpCode::Divu => {
            if b == 0 {
                (0, true)
            } else {
                (a / b, false)
            }
        }
        OpCode::Remu => {
            if b == 0 {
                (0, true)
            } else {
                (a % b, false)
            }
        }
    }
}

fn forwarding_map(forwards: &[Forwarding], include_exceptions: bool) -> HashMap<usize, Forwarding> {
    let mut m = HashMap::new();
    for f in forwards {
        if !include_exceptions && f.exception {
            continue;
        }
        m.insert(f.dest_phys, *f);
    }
    m
}

fn commit_stage(state: &mut ProcessorState) -> bool {
    // Returns true if an exception is detected at commit (enter exception mode this cycle)
    let mut retired = 0usize;

    while retired < 4 {
        let Some(front) = state.active_list.front() else { break };
        if !front.done {
            break;
        }
        if front.exception {
            // will enter exception mode next state
            return true;
        }

        let entry = state.active_list.pop_front().unwrap();
        // recycle old destination physical register
        state.free_list.push_back(entry.old_destination);
        retired += 1;
    }

    false
}

fn issue_stage(state: &ProcessorState, forwards: &[Forwarding]) -> (Vec<AluSlot>, Vec<usize>) {
    // Exception results do not provide a usable value for dependent instructions.
    let fwd_by_tag = forwarding_map(forwards, false);

    // Determine ready entries
    let mut ready: Vec<(u64, usize)> = Vec::new(); // (pc, index)
    for (idx, e) in state.integer_queue.iter().enumerate() {
        let op_a_ready = e.op_a_is_ready || fwd_by_tag.contains_key(&e.op_a_reg_tag);
        let op_b_ready = e.op_b_is_ready || fwd_by_tag.contains_key(&e.op_b_reg_tag);
        if op_a_ready && op_b_ready {
            ready.push((e.pc, idx));
        }
    }

    ready.sort_by_key(|(pc, _)| *pc);
    ready.truncate(4);

    // Build issued slots in PC order
    let mut issued_slots = Vec::new();
    for (_, idx) in ready.iter() {
        let e = &state.integer_queue[*idx];
        let a = if e.op_a_is_ready {
            e.op_a_value
        } else {
            fwd_by_tag.get(&e.op_a_reg_tag).unwrap().value
        };
        let b = if e.op_b_is_ready {
            e.op_b_value
        } else {
            fwd_by_tag.get(&e.op_b_reg_tag).unwrap().value
        };
        let (value, exc) = compute_alu_result(e.op, a, b);
        issued_slots.push(AluSlot {
            pc: e.pc,
            dest_phys: e.dest_register,
            value,
            exception: exc,
        });
    }

    let issued_indices: Vec<usize> = ready.into_iter().map(|(_, idx)| idx).collect();
    (issued_slots, issued_indices)
}

fn rename_dispatch_stage(state: &mut ProcessorState, forwards: &[Forwarding], iq_len_before_issue: usize) -> bool {
    // Returns true if accepted decoded instructions (no backpressure)
    let n = state.decoded_instructions.len();
    if n == 0 {
        return true;
    }

    if state.free_list.len() < n {
        return false;
    }
    if state.active_list.len() + n > 32 {
        return false;
    }
    // IMPORTANT: issued IQ entries are removed only in the next cycle, so R&D cannot
    // rely on Issue freeing IQ slots in the same cycle.
    if iq_len_before_issue + n > 32 {
        return false;
    }

    // Exception results do not provide a usable value for dependent instructions.
    let fwd_by_tag = forwarding_map(forwards, false);

    for inst in state.decoded_instructions.iter() {
        let dest_arch = inst.dest as usize;
        let old_dest_phys = state.register_map_table[dest_arch];
        let new_dest_phys = state.free_list.pop_front().unwrap();

        // Operand A
        let op_a_tag = state.register_map_table[inst.op_a as usize];
        let (op_a_is_ready, op_a_value) = if !state.busy_bit_table[op_a_tag] {
            (true, state.physical_register_file[op_a_tag])
        } else if let Some(f) = fwd_by_tag.get(&op_a_tag) {
            (true, f.value)
        } else {
            (false, 0)
        };

        // Operand B
        let (op_b_is_ready, op_b_tag, op_b_value) = match inst.op_b {
            OperandB::Imm(imm) => (true, 0usize, signext_imm_to_u64(imm)),
            OperandB::Reg(reg) => {
                let tag = state.register_map_table[reg as usize];
                if !state.busy_bit_table[tag] {
                    (true, tag, state.physical_register_file[tag])
                } else if let Some(f) = fwd_by_tag.get(&tag) {
                    (true, tag, f.value)
                } else {
                    (false, tag, 0)
                }
            }
        };

        // Update RMT and busy bit
        state.register_map_table[dest_arch] = new_dest_phys;
        state.busy_bit_table[new_dest_phys] = true;

        // Allocate Active List entry
        state.active_list.push_back(ActiveListEntry {
            done: false,
            exception: false,
            logical_destination: inst.dest,
            old_destination: old_dest_phys,
            pc: inst.pc,
            new_destination: new_dest_phys,
        });

        // Allocate IQ entry
        state.integer_queue.push(IntegerQueueEntry {
            dest_register: new_dest_phys,
            op_a_is_ready,
            op_a_reg_tag: op_a_tag,
            op_a_value,
            op_b_is_ready,
            op_b_reg_tag: op_b_tag,
            op_b_value,
            op_code: inst.op.as_str().to_string(),
            pc: inst.pc,
            op: inst.op,
        });
    }

    // Clear decoded register for next state
    state.decoded_instructions.clear();
    state.decoded_pcs.clear();
    true
}

fn fetch_decode_stage(state: &mut ProcessorState, program: &[Instruction], backpressure: bool) {
    if backpressure {
        return;
    }
    if state.exception {
        return;
    }
    if state.pc >= program.len() as u64 {
        return;
    }

    let mut pcs = Vec::new();
    let mut insts = Vec::new();
    for _ in 0..4 {
        if state.pc >= program.len() as u64 {
            break;
        }
        let inst = program[state.pc as usize];
        pcs.push(inst.pc);
        insts.push(inst);
        state.pc += 1;
    }

    state.decoded_pcs = pcs;
    state.decoded_instructions = insts;
}

fn apply_forwarding(state: &mut ProcessorState, forwards: &[Forwarding]) {
    // Update PRF and busy table
    for f in forwards {
        if !f.exception {
            state.physical_register_file[f.dest_phys] = f.value;
            state.busy_bit_table[f.dest_phys] = false;
        }
    }

    // Update ActiveList done/exception by PC
    for f in forwards {
        for e in state.active_list.iter_mut() {
            if e.pc == f.pc {
                e.done = true;
                if f.exception {
                    e.exception = true;
                }
                break;
            }
        }
    }

    // Update IQ operands (exception results do not wake dependents)
    for f in forwards {
        if f.exception {
            continue;
        }
        for iq in state.integer_queue.iter_mut() {
            if !iq.op_a_is_ready && iq.op_a_reg_tag == f.dest_phys {
                iq.op_a_is_ready = true;
                iq.op_a_value = f.value;
            }
            if !iq.op_b_is_ready && iq.op_b_reg_tag == f.dest_phys {
                iq.op_b_is_ready = true;
                iq.op_b_value = f.value;
            }
        }
    }
}

fn advance_alu(state: &mut ProcessorState, issued: Vec<AluSlot>) {
    // Shift stage1 -> stage2, clear stage2
    for alu in 0..4 {
        state.alu_pipeline[alu][1] = state.alu_pipeline[alu][0];
        state.alu_pipeline[alu][0] = None;
    }

    for (i, slot) in issued.into_iter().enumerate() {
        if i >= 4 {
            break;
        }
        state.alu_pipeline[i][0] = Some(slot);
    }
}

fn reset_iq_and_exec(state: &mut ProcessorState) {
    state.integer_queue.clear();
    state.alu_pipeline = [[None; 2]; 4];
}

fn exception_recovery(state: &mut ProcessorState) {
    // If we scheduled clearing from previous cycle, clear now and exit exception mode.
    if state.exception_will_clear {
        state.exception = false;
        state.exception_will_clear = false;
        return;
    }

    // Roll back up to 4 from bottom
    let mut rolled = 0usize;
    while rolled < 4 {
        let Some(entry) = state.active_list.pop_back() else { break };

        let logical = entry.logical_destination as usize;
        let old = entry.old_destination;
        let new = entry.new_destination;

        state.register_map_table[logical] = old;
        state.busy_bit_table[new] = false;
        state.free_list.push_back(new);

        rolled += 1;
    }

    if state.active_list.is_empty() {
        state.exception_will_clear = true;
    }
}

pub fn propagate_one_cycle(state: &mut ProcessorState, program: &[Instruction]) {
    let forwards = state.current_forwarding();

    if state.exception {
        // In exception mode: no fetch/decode, IQ and exec are already reset at entry.
        exception_recovery(state);
        // PC stays at 0x10000
        state.pc = 0x10000;
        return;
    }

    // Commit stage
    let enter_exception = commit_stage(state);

    if enter_exception {
        // Find the excepting instruction at the head (Done + Exception)
        let pc = state.active_list.front().map(|e| e.pc).unwrap_or(0);
        state.exception = true;
        state.exception_pc = pc;
        state.exception_will_clear = false;

        // Fetch/decode reacts same cycle (next state): PC set to 0x10000 and decoded cleared
        state.pc = 0x10000;
        state.decoded_pcs.clear();
        state.decoded_instructions.clear();

        reset_iq_and_exec(state);
        return;
    }

    // Issue stage (selection uses current IQ; removal happens in next state)
    let iq_len_before_issue = state.integer_queue.len();
    let (issued, mut issued_indices) = issue_stage(state, &forwards);

    // Rename & dispatch stage (uses freed regs from commit and IQ slots freed by issue)
    let accepted = rename_dispatch_stage(state, &forwards, iq_len_before_issue);
    let backpressure = !accepted;

    // Fetch & decode stage
    fetch_decode_stage(state, program, backpressure);

    // Apply forwarding to next state (PRF/busy/IQ/AL done bits)
    apply_forwarding(state, &forwards);

    // Advance ALU pipeline with issued
    advance_alu(state, issued);

    // Remove issued entries from IQ for next state (after R&D allocation decisions)
    if !issued_indices.is_empty() {
        issued_indices.sort_unstable();
        issued_indices.dedup();
        issued_indices.sort_unstable_by(|a, b| b.cmp(a));
        for idx in issued_indices {
            if idx < state.integer_queue.len() {
                state.integer_queue.remove(idx);
            }
        }
    }
}

