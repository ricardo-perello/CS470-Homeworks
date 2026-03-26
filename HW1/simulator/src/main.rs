mod parser;
mod pipeline;
mod state;
mod types;

use std::env;
use std::fs;
use std::path::PathBuf;

use crate::parser::parse_program;
use crate::state::ProcessorState;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: {} <input.json> <output.json>", args[0]);
        std::process::exit(2);
    }

    let input_path = PathBuf::from(&args[1]);
    let output_path = PathBuf::from(&args[2]);

    let input_data = fs::read_to_string(&input_path)?;
    let program = parse_program(&input_data)?;

    let mut state = ProcessorState::new();
    let mut log = Vec::new();
    log.push(state.snapshot());

    while state.has_work(program.len() as u64) {
        state.propagate(&program);
        log.push(state.snapshot());
    }

    let out = serde_json::to_string_pretty(&log)?;
    fs::write(&output_path, out)?;
    Ok(())
}
