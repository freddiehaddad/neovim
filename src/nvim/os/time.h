#pragma once

#include <stdbool.h>
#include <stddef.h>  // IWYU pragma: keep
#include <stdint.h>
#include <time.h>  // NOLINT(misc-header-include-cycle) IWYU pragma: keep

#include "nvim/os/time_defs.h"  // IWYU pragma: keep

uint64_t os_hrtime(void);
int64_t os_realtime(void);
uint64_t os_now(void);
void os_delay(uint64_t ms, bool ignoreinput);
void os_sleep(uint64_t ms);
struct tm *os_localtime_r(const time_t *restrict clock, struct tm *restrict result);
struct tm *os_localtime(struct tm *result);
char *os_ctime_r(const time_t *restrict clock, char *restrict result, size_t result_len,
                 bool add_newline);
char *os_ctime(char *result, size_t result_len, bool add_newline);
char *os_strptime(const char *str, const char *format, struct tm *tm);
Timestamp os_time(void);
