#include "auto/config.h"  // IWYU pragma: keep
#include "auto/versiondef_git.h"
#include "nvim/main.h"
#include "nvim/os/os_defs.h"

#ifdef MSWIN
# ifndef _UCRT
#  error UCRT is the only supported C runtime on windows
# endif
#endif

// for ":version", ":intro", and "nvim --version"
#ifndef NVIM_VERSION_MEDIUM
# define NVIM_VERSION_MEDIUM "v0.13.0-dev"
#endif
#define NVIM_VERSION_LONG "NVIM " NVIM_VERSION_MEDIUM  // NOLINT(bugprone-suspicious-missing-comma)

char *longVersion = NVIM_VERSION_LONG;
#ifdef NVIM_VERSION_BUILD
const char *nvim_version_build = NVIM_VERSION_BUILD;
#else
const char *nvim_version_build = NULL;
#endif

#ifndef MAKE_LIB
int main(int argc, char **argv)
{
  return nvim_main(argc, argv);
}
#endif
