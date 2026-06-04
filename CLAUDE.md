# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

This is an EPFL CS-470 Advanced Computer Architecture course repository with **two homeworks focused on instruction-level parallelism**:

- **HW1**: Build a **cycle-accurate simulator** for an out-of-order (OoO) CPU called **OoO470** (Rust). Simulates a 4-wide superscalar processor with register renaming, speculative execution, and precise exception handling.
- **HW2**: Implement an **instruction scheduler** for a VLIW (Very Long Instruction Word) architecture (Python). Must schedule independent instructions into execution slots while respecting data dependencies.

**Learning Mode**: Help the student understand the architecture and problem, not just solve it. Guide through concepts, ask clarifying questions, and explain the "why" behind solutions.

---

## Quick Start

### HW1 (Rust OoO Simulator)
From `HW1/` directory:

```bash
./build.sh                    # Compile Rust simulator
./run.sh input.json out.json  # Run on a single test
./runall.sh                   # Run all tests in given_tests/
./testall.sh                  # Compare outputs against reference
python compare.py user_output.json -r output.json  # Compare one test
```

**Key files**:
- `simulator/src/main.rs` — Entry point + output logging loop
- `simulator/src/pipeline.rs` — Cycle-by-cycle simulation logic (where most bugs live)
- `simulator/src/state.rs` — Data structures for CPU state
- `simulator/src/types.rs` — Instruction and queue definitions

### HW2 (Python VLIW Scheduler)
From `HW2/` directory:

```bash
./runall.sh                  # Run all tests
./testall.sh                 # Compare against reference
python compare.py input.json simple_ref.json  # Compare one test
python vliw470.py given_tests/01/input.json out.json  # Run one test
```

**Key files**:
- `simulator/vliw470.py` — Main scheduler implementation (what you write)
- `visualize.html` — Interactive schedule viewer (drag/load a JSON output to debug)

---

## Architecture Concepts (Critical Context)

### HW1: Out-of-Order Execution Pipeline

The OoO470 splits execution into conceptual stages that run in parallel:

1. **Fetch+Decode** — Read up to 4 instructions from memory, decode to microops
2. **Rename+Dispatch** — Eliminate false dependencies via register renaming, enqueue to reservation station
3. **Issue** — Select ready instructions (oldest first), send to ALUs
4. **Execute** — Run on 4 ALUs, **fixed 2-cycle latency per op**
5. **Commit** — Retire up to 4 done instructions in-order, handle exceptions precisely

**Key insight**: The tricky part is that everything runs in **the same cycle**, but in a specific order. One ordering mistake breaks the whole log.

The simulator processes each cycle in this order (see `HW1/Readme.md` "Cycle order used by this simulator"):
- Forwarding → Commit → Issue → Rename → Fetch → Apply forwarding → ALU advance → Remove issued ops

### HW2: Instruction Scheduling for VLIW

A VLIW CPU can issue **multiple instructions per cycle** if they:
- Have **no data dependencies** (one doesn't read a value another writes)
- **Don't fight for execution units** (can't run 5 loads on 4 load units)

Your scheduler must group independent instructions into bundles/cycles.

**Key insight**: This is a **graph coloring / bin-packing problem** — find the minimum number of cycles to schedule all instructions while respecting dependencies.

---

## When Helping with a Bug or Implementation

**Learning-mode approach:**

1. **Ask first**: "What does the test failure show? What part of the pipeline/scheduler do you think is wrong?"
2. **Guide, don't solve**: Point to the relevant code/diagram, ask them to trace through their logic
3. **Explain the "why"**: Why does that ordering matter? Why can't we delay that write?
4. **Suggest tools**: Use the visualizer (`HW1/visualize.html`), diff a passing vs failing test output, or trace a simple 2-instruction example by hand

---

## Testing & Debugging Strategy

### HW1 Debugging

- **Visualizer**: Load any `output.json` into `HW1/visualize.html` to step through state cycle-by-cycle
- **Single test**: Run one small test, generate output, compare carefully against reference
- **Trace by hand**: For 2-3 instruction programs, manually walk the pipeline to verify your code matches
- **Common bugs**: Stage ordering (forwarding timing), busy-bit handling, IQ capacity semantics, exception recovery

### HW2 Debugging

- **Visualizer**: Load schedule JSON into `HW2/simulator/visualize.html` to see which instructions ended up in which slots
- **Test descriptions**: Each test's `desc.txt` explains the constraint (e.g., "loop with carry dependency")
- **Diff outputs**: Use `compare.py` to see which cycle/instruction your schedule disagrees with reference

---

## Repository Structure

```
CS470-Homeworks/
├── HW1/                           # Out-of-order CPU simulator (Rust)
│   ├── Homework_1.pdf             # Problem specification
│   ├── simulator/                 # Rust project root
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── pipeline.rs        # ← Core simulation logic
│   │   │   ├── state.rs
│   │   │   ├── types.rs
│   │   │   └── parser.rs
│   │   ├── Cargo.toml
│   │   └── target/release/simulator  # Compiled binary
│   ├── build.sh, run.sh, runall.sh, testall.sh
│   ├── given_tests/               # Test cases (01-20 or similar)
│   │   └── NN/
│   │       ├── input.json         # Program (array of instruction strings)
│   │       ├── output.json        # Reference output
│   │       └── user_output.json   # Your output (generated by runall.sh)
│   └── visualize.html
│
├── HW2/                           # VLIW instruction scheduler (Python)
│   ├── Homework_2.pdf
│   ├── simulator/
│   │   ├── vliw470.py             # ← Your scheduler implementation
│   │   ├── visualize.html
│   │   ├── memory.json, program.json  # Example files
│   │   └── Readme.md
│   ├── given_tests/               # Test cases (01-17)
│   │   └── NN/
│   │       ├── input.json         # Instructions to schedule
│   │       ├── desc.txt           # Test description
│   │       ├── simple_ref.json    # Reference schedule
│   │       └── pip_ref.json       # Alternative reference (pip scheduling)
│   ├── compare.py                 # Validation script
│   ├── runall.sh, testall.sh
│   └── Dockerfile
```

---

## Key Learning Resources in This Repo

- **HW1/Readme.md**: Extensive walkthrough of OoO architecture, cycle semantics, data structures, debugging tips
- **HW2/Readme.md**: Scheduler overview and test format
- **HW2/simulator/Readme.md**: VLIW microarchitecture details
- Each homework PDF: Complete specification and examples

---

## General Development Notes

- **Incremental development**: Start with one small test (e.g., `given_tests/01`), get it passing, then move to the next
- **Visualizers are your friend**: Don't just look at JSON diffs — use the HTML viewers to see what's happening
- **Read the Readme files first**: They contain architectural diagrams, step-by-step cycle ordering, and common pitfalls
- **Docker**: A Dockerfile is provided for each HW to match the grading environment exactly — use it if you hit platform-specific issues

