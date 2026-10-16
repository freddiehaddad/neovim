#pragma once

#include "nvim/types_defs.h"

typedef struct {
  schar_T data;
  sattr_T attr;
} UCell;

typedef struct {
  int row, col;
  int width, height;
  UCell **cells;
} UGrid;

#define UGRID_FOREACH_CELL(grid, row, startcol, endcol, code) \
  do { \
    UCell *row_cells = (grid)->cells[row]; \
    for (int curcol = startcol; curcol < endcol; curcol++) { \
      UCell *cell = row_cells + curcol; \
      (void)(cell); \
      code; \
    } \
  } while (0)

void ugrid_init(UGrid *grid);
void ugrid_free(UGrid *grid);
void ugrid_resize(UGrid *grid, int width, int height);
void ugrid_clear(UGrid *grid);
void ugrid_clear_chunk(UGrid *grid, int row, int col, int endcol, sattr_T attr);
void ugrid_goto(UGrid *grid, int row, int col);
void ugrid_scroll(UGrid *grid, int top, int bot, int left, int right, int count);
