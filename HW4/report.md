# HW4 — Spectre Attack: Report

Ricardo Perello (rperellomas@gmail.com)

## 1. Training the branch predictors

**Local predictor (the PHT entry for the `if (x < array1_size)` branch).**
For every byte to leak I issue `ROUNDS = 30` calls to `victim_function`.
Five out of every six calls use a legitimate `training_x = tries %
array1_size`, so the branch is taken and the PHT entry indexed by that PC
saturates toward *taken*. The sixth call carries the malicious `x`, so the
predictor has just seen a run of taken outcomes before the attack access.

**Global predictor (BHR / correlation).**
Before each victim call I run a deterministic fixed-length loop
(`fill_branch_history`, currently 64 iterations). This creates the same
recent branch history before training and attack calls, so the correlated
predictor indexes the same PHT entries. To avoid adding a data-dependent
branch when selecting the payload, I choose `training_x` vs. `malicious_x`
with bit-level predication rather than `if`/`?:`:

```c
slot_mask = ((j % TRAIN_RATIO) - 1) & ~0xFFFF;
slot_mask = (slot_mask | (slot_mask >> 16));
slot_mask &= attack_mask;
x = training_x ^ (slot_mask & (malicious_x ^ training_x));
```

During the attack phase this evaluates to `malicious_x` exactly when
`j % 6 == 0`; during the control phase `attack_mask` forces every call to
use `training_x`.

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
  before each round; afterwards each is timed with `rdtscp`. The timing
  helper fences before and after the timestamp/load sequence so the OoO
  core cannot move the measured read across the timestamps.
- **Runtime threshold calibration.** On the first call to `attack`, the
  code measures cached and flushed accesses to `array2` and sets the hit
  threshold to `cached_avg + 15` cycles, with conservative bounds and an
  80-cycle fallback. On the local x86 run this calibrated to about
  62-63 cycles.
- **Prefetcher-hostile reload order.** The reload loop visits indices
  in the order `(i * 167 + 13) mod 256`. 167 is coprime with 256, so
  the walk covers all lines but with a stride the L1 stream prefetcher
  cannot lock onto.
- **Filtering the training byte.** During training, `array1[training_x]`
  is legitimately accessed and its corresponding `array2` line is
  always hot. The result tally explicitly skips that index so it does
  not drown the real signal.
- **Attack/control histograms.** Each try has an attack phase and a
  training-only control phase. Returned scores are always
  background-subtracted (`attack_count - control_count`) so systematic
  false hits are not reported as high confidence.
- **Printable confidence filter.** The supplied driver leaks a printable C
  string, so the final selector prefers a strong printable candidate from
  the background-subtracted histogram. "Strong" currently means at least
  100 excess hits over the control histogram; otherwise the byte value
  falls back to the raw attack histogram, but the reported score for that
  byte is still background-subtracted.
- **Full statistical budget.** The code uses all `TRIES = 3000` rounds per
  byte. Early exit was removed because the raw histogram converges
  reliably, while confidence separation benefits from the full control
  baseline.
- **Optimization guards.** At higher optimisation levels GCC can restructure
  code shape and hoist constants the attack depends on. The submitted
  source marks `victim_function`, `fill_branch_history`, and `attack` as
  noinline, asks GCC to compile those functions with `optimize("O0")`, and
  makes `temp` volatile to preserve the intended memory accesses.

## 4. Other problems / lessons learned

- **Initial false positives from the timing threshold.** The original
  fixed 80-cycle threshold and insufficient serialization made many
  unrelated lines look like hits. Adding `lfence` around `rdtscp` and
  calibrating the threshold fixed the raw byte recovery on the local
  machine.
- **Noisy second-best scores.** Even with calibration, some unrelated
  cache lines repeatedly appeared in the low-latency tail. The
  training-only control histogram removes most of that background from
  the reported confidence scores. Some bytes can still print `Unclear`
  locally because the template uses a strict 2x score ratio, but the top
  candidate recovers the full secret string in the local smoke test.
- **Initial false positives from the prefetcher.** A linear reload loop
  (`for i in 0..256`) reported nearby lines as hits because the hardware
  prefetcher pulled them in. Switching to the permuted walk reduced this.
- **`temp` and `array2` writes elided at `-O2`.** With optimisations on,
  the compiler can prove `temp` is never read and drop the speculative
  load. Making `temp` volatile and using function-level optimization guards
  prevents this under GCC.
- **Cross-machine caveat.** Spectre-v1 behavior depends heavily on the
  CPU, microcode, kernel mitigations, and virtualization. The local x86
  run recovers `The Magic Words are Squeamish Ossifrage.` as the top
  candidate, but the threshold margin may need retuning on a different
  grading machine.
