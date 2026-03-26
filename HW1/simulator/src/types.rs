use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    Add,
    Sub,
    Mulu,
    Divu,
    Remu,
}

impl OpCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            OpCode::Add => "add",
            OpCode::Sub => "sub",
            OpCode::Mulu => "mulu",
            OpCode::Divu => "divu",
            OpCode::Remu => "remu",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandB {
    Reg(u8),     // architectural register x0..x31
    Imm(i64),    // sign-extended immediate
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub pc: u64,
    pub op: OpCode,
    pub dest: u8,
    pub op_a: u8,
    pub op_b: OperandB,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActiveListEntry {
    #[serde(rename = "Done")]
    pub done: bool,
    #[serde(rename = "Exception")]
    pub exception: bool,
    #[serde(rename = "LogicalDestination")]
    pub logical_destination: u8,
    #[serde(rename = "OldDestination")]
    pub old_destination: usize,
    #[serde(rename = "PC")]
    pub pc: u64,

    #[serde(skip)]
    pub new_destination: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegerQueueEntry {
    #[serde(rename = "DestRegister")]
    pub dest_register: usize,

    #[serde(rename = "OpAIsReady")]
    pub op_a_is_ready: bool,
    #[serde(rename = "OpARegTag")]
    pub op_a_reg_tag: usize,
    #[serde(rename = "OpAValue")]
    pub op_a_value: u64,

    #[serde(rename = "OpBIsReady")]
    pub op_b_is_ready: bool,
    #[serde(rename = "OpBRegTag")]
    pub op_b_reg_tag: usize,
    #[serde(rename = "OpBValue")]
    pub op_b_value: u64,

    #[serde(rename = "OpCode")]
    pub op_code: String,
    #[serde(rename = "PC")]
    pub pc: u64,

    #[serde(skip)]
    pub op: OpCode,
}

#[derive(Debug, Clone, Copy)]
pub struct AluSlot {
    pub pc: u64,
    pub dest_phys: usize,
    pub value: u64,
    pub exception: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Forwarding {
    pub pc: u64,
    pub dest_phys: usize,
    pub value: u64,
    pub exception: bool,
}

