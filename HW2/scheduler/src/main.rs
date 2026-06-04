//! CLI entry point for the VLIW470 scheduler.
//!
//! Usage:
//!   scheduler <input.json> <simple_out.json> <pip_out.json>

use std::fs;

use anyhow::{Context, Result};

mod alloc_b;
mod alloc_r;
mod deps;
mod emit;
mod ir;
mod parser;
mod prepare_loop;
mod schedule_pip;
mod schedule_simple;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        anyhow::bail!(
            "usage: {} <input.json> <simple_out.json> <pip_out.json>",
            args.first().map(String::as_str).unwrap_or("scheduler")
        );
    }

    let input_text = fs::read_to_string(&args[1])
        .with_context(|| format!("reading {}", &args[1]))?;
    let raw: Vec<String> = serde_json::from_str(&input_text).context("parsing input JSON")?;

    let program = parser::parse_program(&raw)?;
    let dep_table = deps::analyze(&program);

    // -- simple (loop) pipeline --
    let (mut simple_sched, simple_placement) =
        schedule_simple::schedule(&program, &dep_table);
    alloc_b::allocate(&mut simple_sched, &program, &dep_table, &simple_placement);
    let simple_json = emit::emit(&simple_sched);
    fs::write(&args[2], serde_json::to_string_pretty(&simple_json)?)
        .with_context(|| format!("writing {}", &args[2]))?;

    // -- pip (loop.pip) pipeline — student implements the three stubs --
    let (mut pip_sched, pip_placement) =
        schedule_pip::schedule(&program, &dep_table);
    alloc_r::allocate(&mut pip_sched, &program, &dep_table, &pip_placement);
    prepare_loop::prepare(&mut pip_sched, &program);
    let pip_json = emit::emit(&pip_sched);
    fs::write(&args[3], serde_json::to_string_pretty(&pip_json)?)
        .with_context(|| format!("writing {}", &args[3]))?;

    Ok(())
}
