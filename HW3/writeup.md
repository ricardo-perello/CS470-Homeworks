# CS-470 Homework 3 — High-Level Synthesis

**Author:** Ricardo Perello Mas
**Date:** 2026-05-20

---

## Conventions used in this writeup

- An operation with latency $L$ issued at cycle $t$ produces its output **at cycle $t+L$**, where it can be consumed by a downstream operation.
- Latencies (from the assignment):
    - Integer: load/store = 1, add = 1, multiply = 4
    - Float:   load/store = 1, add = 3, multiply = 5
- Each array maps to one BRAM with **one memory port** unless explicitly partitioned.
- Pragma syntax follows Vitis HLS 2025.2 (all optional parameters specified explicitly).
- $N$ denotes `ARRAY_SIZE`.
- In Gantt charts: rows are loop iterations, columns are clock cycles. `LD` / `ST` are 1-cycle memory ops; `MUL` and `ADD` span as many columns as their latency.

---

## Section 2.1 — Static HLS

### Kernel 1 — `array[i] = array[i] * 5`

#### Modified code

```c
void kernel1(int array[ARRAY_SIZE]) {
    #pragma HLS array_partition variable=array type=cyclic factor=3 dim=1
    for (int i = 0; i < ARRAY_SIZE; i++) {
        #pragma HLS pipeline II=1 rewind
        int x = array[i];
        array[i] = (x << 2) + x;
    }
}
```

#### Per-iteration dataflow

A single iteration $i$ performs three operations:

1. `LD array[i]`   — 1 cycle
2. `ADD (x << 2) + x` — 1 cycle (constant shift is wiring)
3. `ST array[i]`   — 1 cycle

Per-iteration latency = $1 + 1 + 1 = 3$ cycles (longest path through one iteration body).

#### Scheduling Gantt chart — first 3 iterations (II = 1)

```
cycle:    0    1    2    3    4
iter 0:  LD   ADD  ST
iter 1:       LD   ADD  ST
iter 2:            LD   ADD  ST
```

Read each row left-to-right: `LD` occupies one cycle, the constant multiply is implemented as a 1-cycle add, and `ST` occupies one cycle. With II = 1 a new iteration starts every cycle.

#### Initiation Interval

$II = 1$.

The body needs **2 memory operations per iteration** (1 load + 1 store) but a single-port BRAM only services **1 access per cycle**, so $\mathrm{ResMII}_{\text{naïve}} = 2$. To get II = 1 we apply cyclic partitioning with factor 3:

- Iteration $i$ uses bank $i\bmod 3$ for both its `LD` (at cycle $i$) and its `ST` (at cycle $i+2$).
- At cycle $c$ the active accesses are: `LD` of iteration $c$ on bank $c\bmod 3$, and `ST` of iteration $c-2$ on bank $(c-2)\bmod 3$.
- These banks are always different, so the port constraint is satisfied.
- A cyclic factor of 2 would conflict here, because the shorter shift-add pipeline would make `LD` of iteration $c$ and `ST` of iteration $c-2$ hit the same parity bank.

There is no loop-carried dependence (each iteration touches a distinct address), so $\mathrm{RecMII} = 0$ and the binding II is $\mathrm{ResMII} = 1$ after partitioning.

#### Cycle counts

- **One iteration:** 3 cycles (from `LD` issue to `ST` completion).
- **Whole task ($N$ iterations, II = 1):** $(N-1) \cdot 1 + 3 = N + 2$ cycles.

#### Resources used

| Resource     | Count | Notes |
|--------------|-------|-------|
| Multipliers  | 0     | Constant multiply is rewritten |
| Adders       | 1     | Computes `(x << 2) + x` |
| BRAM ports for `array` | 3 | After `array_partition cyclic factor=3`, one port per bank |

#### Optimizations and trade-offs

| Optimization | Why | Downside |
|---|---|---|
| `pipeline II=1` | Overlaps consecutive iterations. Without it, each iteration serializes (3 cycles each, total $3N$). | Costs registers between pipeline stages. |
| Replace `x * 5` with `(x << 2) + x` | Removes the 4-cycle integer multiplier and shortens the pipeline fill/drain from 6 cycles to 3 cycles. | Uses one adder; relies on constant shifts being wiring. |
| `array_partition cyclic factor=3` | Gives enough memory banks for the `LD` of iteration $i$ and the `ST` of iteration $i-2$ to proceed in the same cycle. | Uses three BRAM banks instead of one. A lower-area alternative is keeping the multiplier and using factor 2, which still has II = 1 but total latency $N+5$. |

---

### Kernel 2 — `array[i] = array[i-1] + array[i-2] * array[i-3]`

#### Modified code

```c
void kernel2(int array[ARRAY_SIZE]) {
    // Preload the three-element sliding window from memory
    // (cost ignored per assignment: outside the loop).
    int v0 = array[0];  // plays the role of array[i-3]
    int v1 = array[1];  // plays the role of array[i-2]
    int v2 = array[2];  // plays the role of array[i-1]

    for (int i = 3; i < ARRAY_SIZE; i++) {
        #pragma HLS pipeline II=1 rewind
        int result = v2 + v1 * v0;
        array[i] = result;
        // Shift the window: every register update is a free wire,
        // no memory traffic involved.
        v0 = v1;
        v1 = v2;
        v2 = result;
    }
}
```

#### Per-iteration dataflow (after the rewrite)

The naive code performs **3 loads + 1 multiply + 1 add + 1 store** per iteration. With a single BRAM port the three loads would alone push $\mathrm{ResMII}$ to 3, and on top of that there is a hard loop-carried dependence: iteration $i$ writes `array[i]`, which iteration $i+1$ reads as `array[(i+1)-1]`. The synthesiser would conservatively wait for that store to commit before issuing the next read, blowing II up to ~9 cycles.

Key observation: **every value the loop reads, it has already read or computed in a recent iteration**. So we keep a 3-deep sliding window in registers (`v0, v1, v2`) and only ever store to memory. After the rewrite:

1. `MUL v1 · v0`  — 4 cycles
2. `ADD v2 + MUL` — 1 cycle (carries `v2 = previous result`)
3. `ST array[i]` — 1 cycle
4. Shift the window: `v0 ← v1; v1 ← v2; v2 ← result` (free — just wires/flip-flops).

Per-iteration latency = $4 + 1 + 1 = 6$ cycles.

#### Scheduling Gantt chart — first 3 iterations (II = 1)

```
cycle:    0    1    2    3    4    5    6    7    8
iter 0:  MUL  ──── ──── ──── ADD  ST
iter 1:       MUL  ──── ──── ──── ADD  ST
iter 2:            MUL  ──── ──── ──── ADD  ST
```

The carried dependence is the green arrow `ADD_i → ADD_{i+1}` (i.e. `result_i` feeds the next `v2`). It has length 1 cycle ⇒ RecMII = 1.

#### Initiation Interval

$II = 1$.

- **ResMII** — exactly one ST per iteration on a single port ⇒ 1.
  No more loads inside the loop, so the BRAM port is otherwise idle.
- **RecMII** — the carried dependence flows `result_i → v2 ← result_i → ADD_{i+1}`. Path length = latency of the `ADD` itself = 1 cycle ⇒ 1.
- $II = \max(1, 1) = 1$.

Note: the `MUL` of iteration $i+1$ uses the **previous** window's `v1, v2` (which become the new `v0, v1`), so the multiplier chain has **no carried dependence** — it is fully pipelined despite its 4-cycle latency.

#### Cycle counts

- Loop runs from $i = 3$ to $i = N-1$ ⇒ $N - 3$ iterations.
- **One iteration:** 6 cycles.
- **Whole task:** $(N - 3 - 1) \cdot 1 + 6 = N + 2$ cycles.

#### Resources used

| Resource     | Count | Notes |
|--------------|-------|-------|
| Multipliers  | 1     | Pipelined, busy every cycle in steady state |
| Adders       | 1     | The 1-cycle integer add on the recurrence |
| BRAM ports for `array` | 1 | Only stores remain inside the loop |
| Registers    | 3 (32-bit) | The sliding window `v0, v1, v2` |

#### Optimizations and trade-offs

| Optimization | Why | Downside |
|---|---|---|
| **Sliding-window registers** for `array[i-1], array[i-2], array[i-3]` | Eliminates the three loads per iteration, frees the BRAM port, and — crucially — breaks the synthesised RAW dependence between `ST array[i]` and `LD array[i]` in the next iteration. | Requires preloading the first three elements before the loop (free here) and a small register file (3 × word-width). |
| `pipeline II=1` | Issues a new MUL each cycle so the 4-cycle multiplier is fully utilised. | Standard pipeline register cost. |
| **No `unroll`** | An unroll by factor 2 would multiply the multiplier and adder count by 2 with no II benefit (RecMII is already 1) — pure waste. | — |

A subtle pitfall: in the unmodified code the synthesiser sees `array[i-1]` and *might* prove that index `i-1` always differs from index `i` and forward through registers automatically. In practice Vitis HLS does not infer the sliding window from such code — it needs to be written explicitly.

---

### Kernel 3 — Histogram: `hist[index[i]] = hist[index[i]] + weight[i]`

#### Modified code

```c
void kernel3(float hist[ARRAY_SIZE],
             float weight[ARRAY_SIZE],
             int   index[ARRAY_SIZE]) {
    for (int i = 0; i < ARRAY_SIZE; ++i) {
        #pragma HLS pipeline II=5 rewind
        hist[index[i]] = hist[index[i]] + weight[i];
    }
}
```

#### Per-iteration dataflow

Operations per iteration:

1. `LD index[i]`        — 1 cycle (`index` port)
2. `LD weight[i]`       — 1 cycle (`weight` port, in parallel with the index load)
3. `LD hist[index[i]]`  — 1 cycle (`hist` port, must wait for the index value)
4. `FADD hist + weight` — 3 cycles (float add)
5. `ST hist[index[i]]`  — 1 cycle (`hist` port)

Per-iteration latency:

| cycle | op |
|---|---|
| 0 | `LD index[i]`, `LD weight[i]` (parallel, different arrays = different ports) |
| 1 | `LD hist[index[i]]` (now we have the address) |
| 2..4 | `FADD` |
| 5 | `ST hist[index[i]]` |

Latency = 6 cycles.

#### Scheduling Gantt chart — first 3 iterations (II = 5)

```
cycle:    0    1    2    3    4    5    6    7    8    9   10   11   12   13   14   15
iter 0:  LDi  LDh  FADD ──── ──── ST
         LDw
iter 1:                           LDi  LDh  FADD ──── ──── ST
                                  LDw
iter 2:                                                     LDi  LDh  FADD ──── ──── ST
                                                            LDw
```

Legend: `LDi = LD index[i]`, `LDw = LD weight[i]`, `LDh = LD hist[index[i]]`.

#### Initiation Interval

$II = 5$.

The key issue is that the memory dependence through `hist` is not generally false. If `index[i] == index[i+1]`, iteration $i+1$ must read the value written by iteration $i`; otherwise the histogram update loses one of the weights. Since the assignment does not state that the indices are unique, we must preserve this possible RAW dependence.

With the safe schedule:

- **ResMII** — `hist` is hit twice per iteration (1 LD + 1 ST) but its BRAM has a single port. That gives $\mathrm{ResMII}_{\text{hist}} = 2$.
- `weight` and `index` each see 1 LD per iteration ⇒ no bottleneck.
- **RecMII** — the worst-case same-bin recurrence is `LD hist -> FADD -> ST hist -> next LD hist`. With the schedule above, the store of iteration $i$ occurs at cycle $i+5$, and the next iteration's `hist` load can occur at cycle $i+6$. Thus the next loop start is 5 cycles after the previous one.
- $II = \max(2, 5) = 5$.

Could we use `#pragma HLS dependence variable=hist type=inter direction=RAW dependent=false` to push lower? Only under an external precondition, such as all overlapping iterations targeting distinct bins. Under that precondition the carried dependence disappears and the single `hist` port would bind the loop at II = 2. For a general histogram, however, that pragma changes the program's meaning when indices repeat, so I do not use it in the correctness-preserving solution.

#### Cycle counts

- **One iteration:** 6 cycles.
- **Whole task ($N$ iterations, II = 5):** $(N - 1) \cdot 5 + 6 = 5N + 1$ cycles.

#### Resources used

| Resource    | Count | Notes |
|-------------|-------|-------|
| Multipliers | 0     | — |
| Float adders | 1   | The 3-cycle FADD, pipelined |
| BRAM ports for `hist`   | 1 (2 ops/iter) | True dependence possible when indices repeat |
| BRAM ports for `weight` | 1 (1 op/iter)  | — |
| BRAM ports for `index`  | 1 (1 op/iter)  | — |

#### Optimizations and trade-offs

| Optimization | Why | Downside |
|---|---|---|
| `pipeline II=5` | Allows the independent `index` and `weight` loads of the next iteration to overlap with the previous iteration's final `hist` store, while preserving same-bin correctness. | The true histogram recurrence still dominates; speedup over the fully sequential loop is small. |
| **Do not use `dependence false` by default** | Repeated indices are legal for a histogram, so the inter-iteration RAW through `hist` may be real. | Gives up the conditional II = 2 schedule that would be valid only for a no-collision input contract. |
| **Cannot safely partition `hist`** | Indices come from `index[i]` at runtime, so static cyclic/block partitioning cannot rule out two overlapping iterations hitting the same bank/bin. | — |

---

### Kernel 4 — `array[offset] = array[offset] − index[i]·array[i] + index[i]·array[i+1]`

#### Modified code

```c
void kernel4(int array[ARRAY_SIZE], int index[ARRAY_SIZE], int offset) {
    // Hoist the loop-carried accumulator out of memory.
    int acc = array[offset];
    // Bootstrap the sliding-window register for array[i].
    int a_curr = array[offset + 1];

    for (int i = offset + 1; i < ARRAY_SIZE - 1; ++i) {
        #pragma HLS pipeline II=1 rewind
        int a_next = array[i + 1];          // one LD per iteration
        int idx    = index[i];              // one LD on the other port
        int diff   = a_next - a_curr;       // SUB
        int delta  = idx * diff;            // MUL — only one per iteration
        acc        = acc + delta;           // recurrence on acc
        a_curr     = a_next;                // shift the register
    }

    array[offset] = acc;                    // single ST outside the loop
}
```

#### Per-iteration dataflow (after the rewrite)

The original body has **four loads** (`array[offset]`, `index[i]`, `array[i]`, `array[i+1]`), **two multiplies**, **two adds/subtracts**, and **one store back to `array[offset]`** — and every iteration round-trips the accumulator through memory. Three rewrites collapse this:

1. **Hoist `array[offset]` to a scalar `acc`.** The same address is read and written every iteration ⇒ the synthesiser would chain LD→FADD→ST→LD across iterations (RecMII ≈ 7). Lifting the accumulator into a register breaks that chain — only the `ADD acc + delta` cycle remains.
2. **Sliding register for `array[i]`.** Iteration $i+1$ reads `array[(i+1)] = array[i+1]`, which iteration $i$ already loaded. Carry it forward in `a_curr`.
3. **Factor the two multiplies.** $-idx \cdot a_{i} + idx \cdot a_{i+1} = idx \cdot (a_{i+1} - a_{i})$ — one multiply, one extra subtract. Subtracts are 1 cycle, multiplies are 4, so we trade a 4-cycle op for a 1-cycle op.

After all three:

1. `LD array[i+1]`, `LD index[i]` (parallel, two different arrays) — 1 cycle
2. `SUB a_next − a_curr` — 1 cycle
3. `MUL idx · diff` — 4 cycles
4. `ADD acc + delta` — 1 cycle (this is the carried op)

Per-iteration latency = $1 + 1 + 4 + 1 = 7$ cycles.

#### Scheduling Gantt chart — first 3 iterations (II = 1)

```
cycle:    0    1    2    3    4    5    6    7    8    9
iter 0:  LDx  SUB  MUL  ──── ──── ──── ADD
         LDi
iter 1:       LDx  SUB  MUL  ──── ──── ──── ADD
              LDi
iter 2:            LDx  SUB  MUL  ──── ──── ──── ADD
                   LDi
```

`LDx = LD array[i+1]`, `LDi = LD index[i]`. The carried dependence is the chain `ADD_i → acc → ADD_{i+1}`, length = 1 cycle ⇒ RecMII = 1.

#### Initiation Interval

$II = 1$.

- **ResMII** — `array` sees 1 LD per iteration, `index` sees 1 LD per iteration. Each has its own port ⇒ ResMII = 1.
- **RecMII** — accumulator path through ADD: latency 1 ⇒ 1.
- $II = \max(1, 1) = 1$.

A nuance about the multiply: even though `MUL` is 4 cycles, it is **not on the carried path** — it consumes `idx` and `diff` of iteration $i$ and feeds only that iteration's `ADD`. With a pipelined multiplier we issue one MUL per cycle and it has no bearing on II.

#### Cycle counts

- Iterations executed: $M = (\text{ARRAY\_SIZE} - 1) - (\text{offset} + 1) = N - \text{offset} - 2$.
- **One iteration:** 7 cycles.
- **Whole task ($M$ iterations, II = 1):** $(M - 1) + 7 = M + 6 = N - \text{offset} + 4$ cycles (the final `array[offset] = acc` store outside the loop is "negligible" per spec).

#### Resources used

| Resource    | Count | Notes |
|-------------|-------|-------|
| Multipliers | 1     | Factored two muls into one (`idx · (a_next - a_curr)`) |
| Adders      | 2     | One SUB inside the body, one ADD on the recurrence |
| BRAM ports for `array` | 1 | One LD per iteration after the rolling register |
| BRAM ports for `index` | 1 | — |
| Registers   | 2 (`acc`, `a_curr`) | Plus pipeline registers |

#### Optimizations and trade-offs

| Optimization | Why | Downside |
|---|---|---|
| **Scalar accumulator** (`acc`) replaces the LD/ST of `array[offset]` | Removes the dominant carried chain (LD → … → ST → LD) and frees the `array` port. Without this, RecMII ≥ ~7. | None functionally — semantically equivalent because no other code writes `array[offset]` during the loop. |
| **Rolling register** `a_curr` for `array[i]` | Halves the load traffic on `array`. Without it, two LDs/iter on a single port would force ResMII = 2. | Tiny: one extra 32-bit register and bootstrap load. |
| **Factor the two multiplies** into `idx · (a_next − a_curr)` | Saves a multiplier (1 mul instead of 2) and a subtract on the critical compute path (we trade 1 MUL = 4 cycles for 1 SUB = 1 cycle). | None — algebraically identical. |
| `pipeline II=1` | Standard. | Standard. |

---

### Kernel 5 — Data-dependent loop exit

```c
float kernel5(float bound, float a[ARRAY_SIZE], float b[ARRAY_SIZE]) {
    int   i   = 0;
    float sum = 0;
    while (sum < bound && i < ARRAY_SIZE) {
        sum = a[i] + b[i];
        i++;
    }
    return sum;
}
```

#### Modified code

```c
float kernel5(float bound, float a[ARRAY_SIZE], float b[ARRAY_SIZE]) {
    int   i   = 0;
    float sum = 0;
    while (sum < bound && i < ARRAY_SIZE) {
        #pragma HLS pipeline II=5 rewind
        sum = a[i] + b[i];
        i++;
    }
    return sum;
}
```

#### Per-iteration dataflow

1. `LD a[i]` and `LD b[i]` in parallel — 1 cycle (different arrays, different ports)
2. `FADD a + b` — 3 cycles
3. `FCMP sum < bound` — 1 cycle (assumed same latency as a 1-cycle integer compare since the assignment does not give a separate latency for compares; treat as combinational/1-cycle)
4. `&&` with `i < N` and branch — free

Per-iteration latency = $1 + 3 + 1 = 5$ cycles.

If the course model treats the final comparison/branch as purely combinational because comparisons are not listed in the latency table, replace this with 4 cycles. I count it as 1 cycle here to be conservative and to make the control-dependence cost explicit.

#### Scheduling Gantt chart — first 3 iterations (static schedule, II = 5)

```
cycle:    0    1    2    3    4    5    6    7    8    9   10   11   12   13   14
iter 0:  LDa  FADD ──── ──── CMP
         LDb
iter 1:                      LDa  FADD ──── ──── CMP
                             LDb
iter 2:                                          LDa  FADD ──── ──── CMP
                                                 LDb
```

Each iteration must finish its compare before the next iteration is allowed to begin: the synthesiser cannot start iteration $i+1$'s loads while iteration $i$'s exit condition is still in flight, because it does not know whether iteration $i+1$ should execute at all.

#### Initiation Interval

$II = 5$.

- **ResMII** — 1 LD per port per iteration on disjoint arrays ⇒ 1.
- **RecMII** — **the loop exit condition is the binding recurrence**. Iteration $i+1$ cannot legally start until iteration $i$'s compare has produced a verdict. Path = `LD → FADD → FCMP` = $1 + 3 + 1 = 5$ cycles ⇒ 5.
- $II = \max(1, 5) = 5$.

The pipeline pragma is included for completeness — Vitis HLS will still report II = 5 because the recurrence dominates. Without the pragma the design would simply run sequentially (5 cycles/iter as well, just without the controller infrastructure for pipelining). There is essentially no static-HLS optimisation beyond confirming this lower bound.

Note that `sum` is **overwritten** every iteration (`sum = a[i] + b[i]`, not `sum += …`). So the only loop-carried value is `i` (trivial increment) and the exit decision itself. The whole bottleneck is control, not data.

#### Cycle counts

Let $K$ be the iteration count at which the loop exits ($K \le N$).

- **One iteration:** 5 cycles.
- **Whole task ($K$ iterations, II = 5):** $5K + (\text{drain}) = 5K + 0$ ≈ $5K$ cycles, since the last iteration's compare is the exit decision itself.

Under the zero-latency-compare convention, the corresponding static count is approximately $4K$.

For comparison, a dynamically scheduled implementation could approach $K + 4$ cycles (see Section 2.2).

#### Resources used

| Resource     | Count | Notes |
|--------------|-------|-------|
| Multipliers  | 0     | — |
| Float adders | 1     | The 3-cycle FADD, pipelined (but only one iteration in flight at a time under static scheduling) |
| Float compare | 1    | The `<` against `bound` |
| BRAM ports for `a` | 1 (1 LD/iter) | — |
| BRAM ports for `b` | 1 (1 LD/iter) | — |

#### Why no further static-HLS optimization is possible

Every other knob (`unroll`, `array_partition`, `dependence false`) either does not apply or does not help:

- `unroll` of a `while` whose iteration count depends on runtime data is unsafe — the synthesiser would need to assume some upper bound and add fix-up code, with no II benefit because the exit decision still serialises iteration groups.
- `array_partition` — already maximal: `a` and `b` are on separate BRAMs, and there is only 1 access per iteration on each.
- `dependence false` — there is no memory dependence to relax; the bottleneck is **control flow**, not memory.

The fundamental obstruction is that a static schedule must, at compile time, decide whether to execute iteration $i+1$ — and it cannot, because the answer depends on runtime data. The only way to overlap iterations is to **speculate** that the loop continues and roll back on misspeculation, which by definition is dynamic HLS (see §2.2).

---

## Section 2.2 — Dynamic HLS for Kernel 5

### The shape of the dynamic circuit

A dynamically scheduled HLS implementation builds a dataflow circuit where each operator fires the moment its inputs are valid, communicating via the SELF (Synchronous Elastic Flow) handshake (valid/ready). The loop body becomes a small dataflow graph with a `Merge` for `i` at the loop entry, a `Branch` at the exit, and `Buff`/`FIFO` insertion to keep the recurrence cycle deadlock-free and high-throughput.

For Kernel 5 the **branch at the bottom is replaced by a Speculator (`Spec. Branch`)**: it predicts "the loop continues" and emits a new iteration's tokens *before* the actual `sum < bound` comparison resolves. `Save` units sit on the boundary where regular tokens enter the speculative region (the `i++` feedback edge, and the `a[i]`/`b[i]` token-issue paths) so that on a misprediction the originals can be re-emitted. `Commit` units sit on the output edges so any misspeculated tokens that have leaked through downstream are squashed before they reach the exit.

### Why this beats the static design

Static HLS has $II = 5$ because each iteration's start is gated on the previous iteration's compare. A speculative dataflow circuit issues new iterations every cycle — the only floor is now the resource-MII (one load per port per iteration ⇒ ResMII = 1). The cycle diagrams in the course slides (the same `do { d = a[i]+b[i]; ... } while (d<x)` example) show exactly this contrast:

- **Non-speculative dynamic schedule:** still ~5 cycles between iteration starts because the `Branch` waits on the compare.
- **Speculative dynamic schedule (II = 1):** iterations begin every cycle. When iteration $K$'s compare finally fires "exit", iterations $K+1$, $K+2$, $K+3$, $K+4$ are already in-flight; the speculator squashes them via the `Commit` units and the loop terminates.

Approximate cycle counts:

- **Static HLS:** $\approx 5K$ cycles.
- **Speculative dynamic HLS:** $\approx K + 4$ cycles in the no-misprediction case (i.e. when the only exit is taken at iteration $K$). The `+4` covers the in-flight iterations that get squashed.

With a zero-latency compare convention, these become roughly $4K$ and $K+3$.

For large $K$ this is close to a **5× speedup** in steady state.

### Where it really shines: a favorable data distribution

Consider an input where `sum < bound` is true for almost every iteration until the very end. Concretely, suppose:

- `bound = 1.0e6`
- `a[i], b[i] ∈ [1.0, 10.0]` for all `i < ARRAY_SIZE - 1`
- The last entries are crafted so that `a[N-1] + b[N-1] > bound`

Then:

- The loop-continuation condition `sum < bound` is **true** for almost every iteration, so the speculator's "continue" prediction is correct $K-1$ times in a row and wrong only once (the final iteration).
- The misprediction cost is bounded: at most 4 in-flight speculative iterations are squashed, regardless of $K$.
- Total cycles $\approx K + 4$, versus $5K$ in static HLS — a near-5× speedup that **grows with $K$**.

Note that `sum = a[i] + b[i]` (assignment, not accumulation) means each iteration's compare is **independent of the previous iterations' compares**. The speculator therefore does not have to predict a chain of branches — it only has to bet that the loop has not yet hit its exit, which is statistically excellent when the exit is rare.

### Where dynamic HLS would not help

The picture flips if `bound` is so small that the loop exits in 1 or 2 iterations: then the speculator always mispredicts, the squash cost dominates, and the design pays for the speculation hardware (Speculator + Save + Commit units, ~1.5–2× area per the course's experimental results) with no throughput win. Average performance is bounded below by the worst-case input, so a workload mix dominated by short-running invocations should keep the static design.

---

## Summary table

| Kernel | Trick(s) | II | Per-iter latency | Total cycles |
|---|---|---|---|---|
| 1 | shift-add + pipeline + cyclic partition factor 3 | 1 | 3 | $N + 2$ |
| 2 | sliding-window registers + pipeline | 1 | 6 | $N + 2$ |
| 3 | correctness-preserving pipeline | 5 | 6 | $5N + 1$ |
| 4 | scalar accumulator + rolling reg + factored mul + pipeline | 1 | 7 | $N - \text{offset} + 4$ |
| 5 (static) | nothing to do — exit cond. dominates | 5 | 5 | $5K$ |
| 5 (dynamic, §2.2) | speculate on exit | 1 | 5 | $K + 4$ |
