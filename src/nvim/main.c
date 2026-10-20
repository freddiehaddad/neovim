#include <stddef.h>
#include <stdint.h>

#include "auto/config.h"  // IWYU pragma: keep
#include "auto/versiondef_git.h"
#include "nvim/api/private/helpers.h"
#include "nvim/main.h"
#include "nvim/os/os_defs.h"

#ifdef MSWIN
# ifndef _UCRT
#  error UCRT is the only supported C runtime on windows
# endif
#endif

#include "api/private/api_metadata.generated.h"

// for ":version", ":intro", and "nvim --version"
#ifndef NVIM_VERSION_MEDIUM
# define NVIM_VERSION_MEDIUM "v0.13.0-dev"
#endif
#define NVIM_VERSION_LONG "NVIM " NVIM_VERSION_MEDIUM  // NOLINT(bugprone-suspicious-missing-comma)

char *longVersion = NVIM_VERSION_LONG;

const uint8_t *api_get_packed_metadata(size_t *len)
{
  *len = sizeof(packed_api_metadata);
  return packed_api_metadata;
}

#ifndef MAKE_LIB
int main(int argc, char **argv)
{
  return nvim_main(argc, argv);
}
#endif
