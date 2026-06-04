# HW4 — Spectre Attack: Report

Ricardo Perello (rperellomas@gmail.com)

## 1. Training the branch predictors

**Local predictor (the PHT entry for the `if (x < array1_size)` branch).**
For every byte to leak we issue `ROUNDS = 30` calls to `victim_function`.
Five out of every six calls use a legitimate `training_x = tries %
array1_size`, so the branch is taken and the PHT entry indexed by that PC
saturates toward *taken*. Only the sixth call carries the malicious `x`,
and by then the predictor confidently mispredicts in our favour.

**Global predictor (BHR / correlation).**
The training loop itself is a deterministic, fixed-length `for (j = 29;
j >= 0; j--)` whose body always executes the same conditional sequence.
This drives a stable, repeatable history into the BHR, so each call to
`victim_function` indexes the *same* PHT slot during training and during
the attack. To avoid corrupting that history I selected
`x = training_x` vs. `malicious_x` with **bit-level predication** rather
than `if`/`?:` — see `attack()`:

```c
x = ((j % TRAIN_RATIO) - 1) & ~0xFFFF;
x = (x | (x >> 16));
x = training_x ^ (x & (malicious_x ^ training_x));
```

This evaluates to `malicious_x` exactly when `j % 6 == 0` and to
`training_x` otherwise, with no data-dependent jump.

## 2. Extending the side channel

Right before each victim call I flush the bounds variable from cache:

```c
_mm_clflush(&array1_size);
```

The comparison `x < array1_size` now stalls on a DRAM load, which keeps
the branch unresolved for hundreds of cycles. During that window the
processor speculatively executes `temp ^= array2[array1[x] * 512]`, and
the dependent load brings the secret-indexed line of `array2` into L1
before the misprediction is squashed.

The short `for (volatile int z = 0; z < 100; z++)` after the flush makes
sure the flush has retired so that the dependent comparison really does
miss.

## 3. Techniques used to improve accuracy

- **Flush+Reload on `array2`.** All 256 candidate lines are flushed
  before each round; afterwards each is timed with `rdtscp` and counted
  as a hit if its access time is below `CACHE_HIT_THRESHOLD`. The
  cycle threshold (default 80) should be re-measured on the grading
  machine — a calibration loop comparing a hot vs. cold read gives the
  right cut-off.
- **Prefetcher-hostile reload order.** The reload loop visits indices
  in the order `(i * 167 + 13) mod 256`. 167 is coprime with 256, so
  the walk covers all lines but with a stride the L1 stream prefetcher
  cannot lock onto.
- **Filtering the training byte.** During training, `array1[training_x]`
  is legitimately accessed and its corresponding `array2` line is
  always hot. The result tally explicitly skips that index so it does
  not drown the real signal.
- **Statistical voting with early exit.** Up to `TRIES = 999` rounds
  per byte. We stop as soon as one candidate's score exceeds twice the
  runner-up's (`results[j] >= 2*results[k] + 5`), which is the same
  criterion `main()` prints as `Success`. This keeps the run fast on
  easy bytes and only spends real time on noisy ones.
- **`-O0` compilation.** At higher optimisation levels GCC restructures
  the branchless predication into a conditional move or jump and can
  hoist constants the attack depends on. `-O0` keeps the code shape
  intact.

## 4. Other problems / lessons learned

- **Initial false positives from the prefetcher.** A linear reload loop
  (`for i in 0..256`) reliably reported the *neighbouring* line as the
  winner because the hardware prefetcher pulled it in. Switching to the
  permuted walk fixed this.
- **`temp` and `array2` writes elided at `-O2`.** With optimisations on,
  the compiler proved `temp` is never read and dropped the speculative
  load. The `temp` global is declared at file scope and `-O0` is used to
  prevent this.
- **`rdtscp` serialisation.** Wrapping the timed access with
  `_mm_mfence()` before `rdtscp` and `_mm_lfence()` after the load makes
  the measurement reproducible; without fences the OoO core sometimes
  reorders the timed instruction around `rdtscp` and the histogram
  becomes bimodal.
- **Cross-architecture caveat.** Development on Apple Silicon (arm64) is
  not representative — `clflush`, `rdtscp`, and the x86 Spectre-v1 PHT
  behaviour are absent. All tuning was done on x86 Linux.
