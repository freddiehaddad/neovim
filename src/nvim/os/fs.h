#pragma once

#include <stddef.h>  // IWYU pragma: keep
#include <stdint.h>  // IWYU pragma: keep
#include <stdio.h>  // IWYU pragma: keep
#include <uv.h>  // IWYU pragma: keep

#include "nvim/os/fs_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

#define DEFINE_FUNC_ATTRIBUTES
#include "nvim/func_attr.h"
#undef DEFINE_FUNC_ATTRIBUTES
#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT int os_chdir(const char *path) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int os_dirname(char *buf, size_t len) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool os_isrealdir(const char *name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool os_isdir(const char *name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int os_nodetype(const char *name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int os_exepath(char *buffer, size_t *size) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool os_can_exe(const char *name, char **abspath, bool use_path) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT int os_open(const char *path, int flags, int mode);
DLLEXPORT FILE *os_fopen(const char *path, const char *flags);
DLLEXPORT int os_set_cloexec(const int fd);
DLLEXPORT int os_close(const int fd);
DLLEXPORT int os_dup(const int fd) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int os_dup_cloexec(const int fd) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int os_open_stdin_fd(void);
DLLEXPORT ptrdiff_t os_read(const int fd, bool *const ret_eof, char *const ret_buf,
                            const size_t size,
                            const bool non_blocking) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT ptrdiff_t os_readv(const int fd, bool *const ret_eof, struct iovec *iov, size_t iov_size,
                             const bool non_blocking) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT ptrdiff_t os_write(const int fd, const char *const buf, const size_t size,
                             const bool non_blocking) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int os_copy(const char *path, const char *new_path, int flags);
DLLEXPORT int os_fsync(int fd);
DLLEXPORT int32_t os_getperm(const char *name);
DLLEXPORT int os_setperm(const char *const name, int perm) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int os_fsetperm(int fd, int perm);
DLLEXPORT void os_copy_xattr(const char *from_file, const char *to_file);
DLLEXPORT vim_acl_T os_get_acl(const char *fname);
DLLEXPORT void os_set_acl(const char *fname, vim_acl_T aclent);
DLLEXPORT void os_free_acl(vim_acl_T aclent);
DLLEXPORT bool os_file_owned(const char *fname) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int os_chown(const char *path, uv_uid_t owner, uv_gid_t group);
DLLEXPORT int os_fchown(int fd, uv_uid_t owner, uv_gid_t group);
DLLEXPORT bool os_path_exists(const char *path);
DLLEXPORT int os_file_settime(const char *path, double atime, double mtime);
DLLEXPORT bool os_file_is_readable(const char *name)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int os_file_is_writable(const char *name)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int os_rename(const char *path, const char *new_path) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int os_mkdir(const char *path, int32_t mode) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int os_mkdir_recurse(const char *const dir, int32_t mode, char **const failed_dir,
                               char **const created)
  FUNC_ATTR_NONNULL_ARG(1, 3) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int os_file_mkdir(char *fname, int32_t mode)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int os_mkdtemp(const char *templ, char *path) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int os_rmdir(const char *path) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool os_scandir(Directory *dir, const char *path) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT const char *os_scandir_next(Directory *dir) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void os_closedir(Directory *dir) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int os_remove(const char *path) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool os_fileinfo(const char *path, FileInfo *file_info) FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT bool os_fileinfo2(const char *path, FileInfo *info) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool os_fileinfo_link(const char *path, FileInfo *file_info) FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT bool os_fileinfo_fd(int file_descriptor, FileInfo *file_info) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool os_fileinfo_id_equal(const FileInfo *file_info_1,
                                    const FileInfo *file_info_2) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void os_fileinfo_id(const FileInfo *file_info, FileID *file_id) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT uint64_t os_fileinfo_inode(const FileInfo *file_info) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT uint64_t os_fileinfo_size(const FileInfo *file_info) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT uint64_t os_fileinfo_hardlinks(const FileInfo *file_info) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT uint64_t os_fileinfo_blocksize(const FileInfo *file_info) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool os_fileid(const char *path, FileID *file_id) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool os_fileid_equal(const FileID *file_id_1,
                               const FileID *file_id_2) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *os_realpath(const char *name, char *buf, size_t len) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT char *os_resolve_shortcut(const char *fname)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_MALLOC;
DLLEXPORT bool os_is_reparse_point_include(const char *path);

#include "nvim/func_attr.h"
