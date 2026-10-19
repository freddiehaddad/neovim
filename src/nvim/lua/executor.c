#include <stddef.h>
#include <stdint.h>

#include "nvim/lua/executor.h"
#include "nvim/macros_defs.h"

typedef struct {
  char *name;
  const uint8_t *data;
  size_t size;
} ModuleDef;

#include "lua/executor.c.generated.h"
#include "lua/vim_module.generated.h"

const void *nlua_get_builtin_modules(size_t *count)
{
  *count = ARRAY_SIZE(builtin_modules);
  return builtin_modules;
}
