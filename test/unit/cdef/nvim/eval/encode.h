#pragma once

#include <string.h>

#include "nvim/eval/typval_defs.h"
#include "nvim/garray_defs.h"
#include "nvim/msgpack_rpc/packer_defs.h"

/// Convert Vimscript value to msgpack string
///
/// @param[out]  packer  Packer to save results in.
/// @param[in]  tv  Dumped value.
/// @param[in]  objname  Object name, used for error message.
///
/// @return OK in case of success, FAIL otherwise.
int encode_vim_to_msgpack(PackerBuffer *packer, typval_T *tv, const char *objname);

/// Convert Vimscript value to :echo output
///
/// @param[out]  gap  garray_T to save results in.
/// @param[in]  tv  Dumped value.
/// @param[in]  objname  Object name, used for error message.
///
/// @return OK in case of success, FAIL otherwise.
int encode_vim_to_echo(garray_T *gap, typval_T *tv, const char *objname);

/// Structure defining state for read_from_list()
typedef struct {
  const list_T *const list;  ///< List being currently read.
  const listitem_T *li;  ///< Item currently read.
  size_t offset;  ///< Byte offset inside the read item.
  size_t li_length;  ///< Length of the string inside the read item.
} ListReaderState;

/// Array mapping values from SpecialVarValue enum to names
extern const char *const encode_bool_var_names[];
extern const char *const encode_special_var_names[];

/// First codepoint in high surrogates block
#define SURROGATE_HI_START 0xD800

/// Last codepoint in high surrogates block
#define SURROGATE_HI_END   0xDBFF

/// First codepoint in low surrogates block
#define SURROGATE_LO_START 0xDC00

/// Last codepoint in low surrogates block
#define SURROGATE_LO_END   0xDFFF

/// First character that needs to be encoded as surrogate pair
#define SURROGATE_FIRST_CHAR 0x10000

int encode_blob_write(void *const data, const char *const buf, const size_t len);
void encode_list_write(void *const data, const char *const buf, const size_t len);
bool encode_vim_list_to_buf(const list_T *const list, size_t *const ret_len, char **const ret_buf);
int encode_read_from_list(ListReaderState *const state, char *const buf, const size_t nbuf,
                          size_t *const read_bytes);
bool encode_check_json_key(const typval_T *const tv);
char *encode_tv2string(typval_T *tv, size_t *len);
char *encode_tv2echo(typval_T *tv, size_t *len);
char *encode_tv2json(typval_T *tv, size_t *len);
ListReaderState encode_init_lrstate(const list_T *const list);
