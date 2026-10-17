#pragma once

#include "nvim/memfile_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// flags for mf_sync()
enum {
  MFS_ALL   = 1,  ///< also sync blocks with negative numbers
  MFS_STOP  = 2,  ///< stop syncing when a character is available
  MFS_FLUSH = 4,  ///< flushed file to disk
  MFS_ZERO  = 8,  ///< only write block 0
};

enum {
  /// Minimal size for block 0 of a swap file.
  /// NOTE: This depends on size of struct block0! It's not done with a sizeof(),
  /// because struct block0 is defined in memline.c (Sorry).
  /// The maximal block size is arbitrary.
  MIN_SWAP_PAGE_SIZE = 1048,
  MAX_SWAP_PAGE_SIZE = 50000,
};

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT memfile_T *mf_open(char *fname, int flags);
DLLEXPORT int mf_open_file(memfile_T *mfp, char *fname);
DLLEXPORT void mf_close(memfile_T *mfp, bool del_file);
DLLEXPORT void mf_close_file(buf_T *buf, bool getlines);
DLLEXPORT void mf_new_page_size(memfile_T *mfp, unsigned new_size);
DLLEXPORT bhdr_T *mf_new(memfile_T *mfp, bool negative, unsigned page_count);
DLLEXPORT bhdr_T *mf_get(memfile_T *mfp, blocknr_T nr, unsigned page_count);
DLLEXPORT void mf_put(memfile_T *mfp, bhdr_T *hp, bool dirty, bool infile);
DLLEXPORT void mf_free(memfile_T *mfp, bhdr_T *hp);
DLLEXPORT int mf_sync(memfile_T *mfp, int flags);
DLLEXPORT void mf_set_dirty(memfile_T *mfp);
DLLEXPORT bool mf_release_all(void);
DLLEXPORT blocknr_T mf_trans_del(memfile_T *mfp, blocknr_T old_nr);
DLLEXPORT void mf_free_fnames(memfile_T *mfp);
DLLEXPORT void mf_set_fnames(memfile_T *mfp, char *fname);
DLLEXPORT void mf_fullname(memfile_T *mfp);
DLLEXPORT bool mf_need_trans(memfile_T *mfp);
