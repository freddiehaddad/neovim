#pragma once

#include <stddef.h>  // IWYU pragma: keep

char *base64_encode(const char *src, size_t src_len);
char *base64_decode(const char *src, size_t src_len, size_t *out_lenp);
