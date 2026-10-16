#pragma once

#include <stdbool.h>
#include <stddef.h>  // IWYU pragma: keep
#include <stdint.h>
#include <string.h>  // IWYU pragma: keep

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/msgpack_rpc/packer_defs.h"

#define mpack_w(b, byte) *(*(b))++ = (char)(byte);
static inline void mpack_w2(char **b, uint32_t v)
{
  *(*b)++ = (char)((v >> 8) & 0xff);
  *(*b)++ = (char)(v & 0xff);
}

static inline void mpack_w4(char **b, uint32_t v)
{
  *(*b)++ = (char)((v >> 24) & 0xff);
  *(*b)++ = (char)((v >> 16) & 0xff);
  *(*b)++ = (char)((v >> 8) & 0xff);
  *(*b)++ = (char)(v & 0xff);
}

static inline void mpack_uint(char **buf, uint32_t val)
{
  if (val > 0xffff) {
    mpack_w(buf, 0xce);
    mpack_w4(buf, val);
  } else if (val > 0xff) {
    mpack_w(buf, 0xcd);
    mpack_w2(buf, val);
  } else if (val > 0x7f) {
    mpack_w(buf, 0xcc);
    mpack_w(buf, val);
  } else {
    mpack_w(buf, val);
  }
}

#define mpack_nil(buf) mpack_w(buf, 0xc0)
static inline void mpack_bool(char **buf, bool val)
{
  mpack_w(buf, 0xc2 | (val ? 1 : 0));
}

static inline void mpack_array(char **buf, uint32_t len)
{
  if (len < 0x10) {
    mpack_w(buf, 0x90 | len);
  } else if (len < 0x10000) {
    mpack_w(buf, 0xdc);
    mpack_w2(buf, len);
  } else {
    mpack_w(buf, 0xdd);
    mpack_w4(buf, len);
  }
}

static inline void mpack_map(char **buf, uint32_t len)
{
  if (len < 0x10) {
    mpack_w(buf, 0x80 | len);
  } else if (len < 0x10000) {
    mpack_w(buf, 0xde);
    mpack_w2(buf, len);
  } else {
    mpack_w(buf, 0xdf);
    mpack_w4(buf, len);
  }
}

static inline size_t mpack_remaining(PackerBuffer *packer)
{
  return (size_t)(packer->endptr - packer->ptr);
}

void mpack_check_buffer(PackerBuffer *packer);
void mpack_uint64(char **ptr, uint64_t i);
void mpack_integer(char **ptr, Integer i);
void mpack_float8(char **ptr, double i);
void mpack_str(String str, PackerBuffer *packer);
void mpack_bin(String str, PackerBuffer *packer);
void mpack_raw(const char *data, size_t len, PackerBuffer *packer);
void mpack_ext(char *buf, size_t len, int8_t type, PackerBuffer *packer);
void mpack_handle(ObjectType type, handle_T handle, PackerBuffer *packer);
void mpack_object(Object *obj, PackerBuffer *packer);
void mpack_object_array(Array arr, PackerBuffer *packer);
void mpack_object_inner(Object *current, Object *container, size_t container_idx,
                        PackerBuffer *packer);
PackerBuffer packer_string_buffer(void);
String packer_take_string(PackerBuffer *buffer);
