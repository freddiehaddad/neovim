#pragma once

#include <stdbool.h>
#include <stddef.h>  // IWYU pragma: keep

#include "mpack/object.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

list_T *decode_create_map_special_dict(typval_T *const ret_tv, const ptrdiff_t len);
typval_T decode_string(const char *const s, const size_t len, bool force_blob,
                       const bool s_allocated);
int json_decode_string(const char *const buf, const size_t buf_len, typval_T *const rettv);
void typval_parser_error_free(mpack_parser_t *parser);
int mpack_parse_typval(mpack_parser_t *parser, const char **data, size_t *size);
int unpack_typval(const char **data, size_t *size, typval_T *ret);
