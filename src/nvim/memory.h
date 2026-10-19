#pragma once

#include <stdbool.h>
#include <stdint.h>  // IWYU pragma: keep
#include <string.h>
#include <time.h>  // IWYU pragma: keep

#include "auto/config.h"
#include "nvim/macros_defs.h"
#include "nvim/memory_defs.h"  // IWYU pragma: keep

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

/// `malloc()` function signature
typedef void *(*MemMalloc)(size_t);

/// `free()` function signature
typedef void (*MemFree)(void *);

/// `calloc()` function signature
typedef void *(*MemCalloc)(size_t, size_t);

/// `realloc()` function signature
typedef void *(*MemRealloc)(void *, size_t);

#ifdef UNIT_TESTING
/// When unit testing: pointer to the `malloc()` function, may be altered
extern MemMalloc mem_malloc;

/// When unit testing: pointer to the `free()` function, may be altered
extern MemFree mem_free;

/// When unit testing: pointer to the `calloc()` function, may be altered
extern MemCalloc mem_calloc;

/// When unit testing: pointer to the `realloc()` function, may be altered
extern MemRealloc mem_realloc;
#endif

#ifdef EXITFREE
/// Indicates that free_all_mem function was or is running
extern bool entered_free_all_mem;
#endif

typedef void *(*MergeSortGetFunc)(void *);
typedef void (*MergeSortSetFunc)(void *, void *);
typedef int (*MergeSortCompareFunc)(const void *, const void *);

extern size_t arena_alloc_count;

#define kv_fixsize_arena(a, v, s) \
  ((v).capacity = (s), \
   (v).items = (void *)arena_alloc(a, sizeof((v).items[0]) * (v).capacity, true))

void *try_malloc(size_t size);
void *verbose_try_malloc(size_t size);
void *xmalloc(size_t size);
void xfree(void *ptr);
void *xcalloc(size_t count, size_t size);
void *xrealloc(void *ptr, size_t size);
void *xmallocz(size_t size);
void *xmemdupz(const void *data, size_t len);
void *xmemcpyz(void *dst, const void *src, size_t len);
size_t xstrnlen(const char *s, size_t n);
char *xstrchrnul(const char *str, char c);
void *xmemscan(const void *addr, char c, size_t size);
void strchrsub(char *str, char c, char x);
void memchrsub(void *data, char c, char x, size_t len);
size_t strcnt(const char *str, char c);
size_t memcnt(const void *data, char c, size_t len);
char *xstpcpy(char *restrict dst, const char *restrict src);
char *xstpncpy(char *restrict dst, const char *restrict src, size_t maxlen);
size_t xstrlcpy(char *restrict dst, const char *restrict src, size_t dsize);
size_t xstrlcat(char *const dst, const char *const src, const size_t dsize);
char *xstrdup(const char *str);
char *xstrdupnul(const char *const str);
void *xmemrchr(const void *src, uint8_t c, size_t len);
char *xstrndup(const char *str, size_t len);
void *xmemdup(const void *data, size_t len);
bool strequal(const char *a, const char *b);
bool strnequal(const char *a, const char *b, size_t n);
void time_to_bytes(time_t time_, uint8_t buf[8]);
void *mergesort_list(void *head, MergeSortGetFunc get_next, MergeSortSetFunc set_next,
                     MergeSortGetFunc get_prev, MergeSortSetFunc set_prev,
                     MergeSortCompareFunc compare);
void arena_free_reuse_blks(void);
ArenaMem arena_finish(Arena *arena);
void *alloc_block(void);
void arena_alloc_block(Arena *arena);
void *arena_alloc(Arena *arena, size_t size, bool align);
void free_block(void *block);
void arena_mem_free(ArenaMem mem);
char *arena_allocz(Arena *arena, size_t size);
char *arena_memdupz(Arena *arena, const char *buf, size_t size);
char *arena_strdup(Arena *arena, const char *str);
void free_all_mem(void);

#define XFREE_CLEAR(ptr) \
  do { \
    /* Take the address to avoid double evaluation. #1375 */ \
    void **ptr_ = (void **)&(ptr); \
    xfree(*ptr_); \
    /* coverity[dead-store] */ \
    *ptr_ = NULL; \
    (void)(*ptr_); \
  } while (0)

#define CLEAR_FIELD(field)  memset(&(field), 0, sizeof(field))
#define CLEAR_POINTER(ptr)  memset((ptr), 0, sizeof(*(ptr)))

#ifndef HAVE_STRNLEN
# define strnlen xstrnlen  // Older versions of SunOS may not have strnlen
#endif

#define STRCPY(d, s)        strcpy((char *)(d), (char *)(s))  // NOLINT(runtime/printf)

// Like strcpy() but allows overlapped source and destination.
#define STRMOVE(d, s)       memmove((d), (s), strlen(s) + 1)
