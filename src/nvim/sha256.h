#pragma once

#include <stdbool.h>
#include <stddef.h>  // IWYU pragma: keep
#include <stdint.h>

#define SHA256_BUFFER_SIZE 64
#define SHA256_SUM_SIZE    32

typedef struct {
  uint32_t total[2];
  uint32_t state[8];
  uint8_t buffer[SHA256_BUFFER_SIZE];
} context_sha256_T;

void sha256_start(context_sha256_T *ctx);
void sha256_update(context_sha256_T *ctx, const uint8_t *input, size_t length);
void sha256_finish(context_sha256_T *ctx, uint8_t digest[SHA256_SUM_SIZE]);
const char *sha256_bytes(const uint8_t *restrict buf, size_t buf_len, const uint8_t *restrict salt,
                         size_t salt_len);
bool sha256_self_test(void);
