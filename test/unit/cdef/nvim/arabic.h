#pragma once

#include <stdbool.h>

#define ARABIC_CHAR(ch)            (((ch) & 0xFF00) == 0x0600)

bool arabic_maycombine(int two);
bool arabic_combine(int one, int two);
int arabic_shape(int c, int *c1p, int prev_c, int prev_c1, int next_c);
