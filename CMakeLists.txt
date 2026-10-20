# CMAKE REFERENCE
#   - intro: https://codingnest.com/basic-cmake/
#   - best practices (3.0+): https://gist.github.com/mbinna/c61dbb39bca0e4fb7d1f73b0d66a4fd1
#   - pitfalls: https://izzys.casa/2019/02/everything-you-never-wanted-to-know-about-cmake/
#   - troubleshooting:
#     - variable_watch https://cmake.org/cmake/help/latest/command/variable_watch.html
#     - verbose output: cmake --build build --verbose

# Version should match the tested CMAKE_URL in .github/workflows/build.yml.
cmake_minimum_required(VERSION 3.16)

project(nvim C)

if(POLICY CMP0135)
  cmake_policy(SET CMP0135 NEW)
endif()

if(XCODE)
  message(FATAL_ERROR [[Xcode generator is not supported. Use "Ninja" or "Unix Makefiles" instead]])
endif()

# Point CMake at any custom modules we may ship
list(APPEND CMAKE_MODULE_PATH "${PROJECT_SOURCE_DIR}/cmake")

include(ExternalProject)
include(GNUInstallDirs)

include(Deps)
include(InstallHelpers)
include(PreventInTreeBuilds)
include(Util)

if(NOT PROJECT_SOURCE_DIR STREQUAL PROJECT_BINARY_DIR)
  # Auto-create a .gitignore in the specified "build" directory.
  file(GENERATE OUTPUT .gitignore CONTENT "*")
endif()

#-------------------------------------------------------------------------------
# User settings
#-------------------------------------------------------------------------------

if(NOT DEPS_IGNORE_SHA)
  set(DEPS_IGNORE_SHA FALSE)
endif()

#-------------------------------------------------------------------------------
# Variables
#-------------------------------------------------------------------------------
set(TOUCHES_DIR ${PROJECT_BINARY_DIR}/touches)
set(VTERM_TEST_FILE ${PROJECT_BINARY_DIR}/test/vterm_test_output)
set(NVIM_BIN_FILE "${CMAKE_BINARY_DIR}/bin/nvim${CMAKE_EXECUTABLE_SUFFIX}")

file(GLOB DOCFILES CONFIGURE_DEPENDS ${PROJECT_SOURCE_DIR}/runtime/doc/*.txt)

if(NOT CI_BUILD)
  set(CMAKE_INSTALL_MESSAGE NEVER)
endif()

if(${CMAKE_VERSION} VERSION_GREATER_EQUAL 3.26)
  set(COPY_DIRECTORY copy_directory_if_different)
else()
  set(COPY_DIRECTORY copy_directory)
endif()

# Prefer our bundled versions of dependencies.
if(DEFINED ENV{DEPS_BUILD_DIR})
  set(DEPS_PREFIX "$ENV{DEPS_BUILD_DIR}/usr" CACHE PATH "Path prefix for finding dependencies")
else()
  set(DEPS_PREFIX "${CMAKE_CURRENT_SOURCE_DIR}/.deps/usr" CACHE PATH "Path prefix for finding dependencies")
  # When running from within CLion or Visual Studio,
  # build bundled dependencies automatically.
  if(NOT EXISTS ${DEPS_PREFIX}
     AND (DEFINED ENV{CLION_IDE}
          OR DEFINED ENV{VisualStudioEdition}))
    message(STATUS "Building dependencies...")
    set(DEPS_BUILD_DIR ${PROJECT_BINARY_DIR}/.deps)
    file(MAKE_DIRECTORY ${DEPS_BUILD_DIR})
    execute_process(
      COMMAND ${CMAKE_COMMAND} -G ${CMAKE_GENERATOR}
        -D CMAKE_TOOLCHAIN_FILE=${CMAKE_TOOLCHAIN_FILE}
        -D CMAKE_BUILD_TYPE=${CMAKE_BUILD_TYPE}
        -D CMAKE_C_COMPILER=${CMAKE_C_COMPILER}
        -D CMAKE_C_FLAGS=${CMAKE_C_FLAGS}
        -D CMAKE_C_FLAGS_DEBUG=${CMAKE_C_FLAGS_DEBUG}
        -D CMAKE_C_FLAGS_MINSIZEREL=${CMAKE_C_FLAGS_MINSIZEREL}
        -D CMAKE_C_FLAGS_RELWITHDEBINFO=${CMAKE_C_FLAGS_RELWITHDEBINFO}
        -D CMAKE_C_FLAGS_RELEASE=${CMAKE_C_FLAGS_RELEASE}
        -D CMAKE_MAKE_PROGRAM=${CMAKE_MAKE_PROGRAM}
        ${PROJECT_SOURCE_DIR}/cmake.deps
      WORKING_DIRECTORY ${DEPS_BUILD_DIR})
    execute_process(
      COMMAND ${CMAKE_COMMAND} --build ${DEPS_BUILD_DIR}
        --config ${CMAKE_BUILD_TYPE})
    set(DEPS_PREFIX ${DEPS_BUILD_DIR}/usr)
  endif()
endif()

list(INSERT CMAKE_PREFIX_PATH 0 ${DEPS_PREFIX})

if(APPLE)
  # If the macOS deployment target is not set manually (via $MACOSX_DEPLOYMENT_TARGET),
  # fall back to local system version. Needs to be done both here and in cmake.deps.
  if(NOT CMAKE_OSX_DEPLOYMENT_TARGET)
    execute_process(COMMAND sw_vers -productVersion
                    OUTPUT_VARIABLE MACOS_VERSION
                    OUTPUT_STRIP_TRAILING_WHITESPACE)
    set(CMAKE_OSX_DEPLOYMENT_TARGET "${MACOS_VERSION}")
  endif()
  message(STATUS "Using deployment target ${CMAKE_OSX_DEPLOYMENT_TARGET}")
endif()

option(ENABLE_LIBINTL "enable libintl" ON)
option(ENABLE_UNIBILIUM "enable unibilium" ON)
option(ENABLE_WASMTIME "enable wasmtime" OFF)

message(STATUS "CMAKE_INSTALL_PREFIX=${CMAKE_INSTALL_PREFIX}")

set_default_buildtype(Debug)

# If not in a git repo (e.g., a tarball) these tokens define the complete
# version string, else they are combined with the result of `git describe`.
set(NVIM_VERSION_MAJOR 0)
set(NVIM_VERSION_MINOR 13)
set(NVIM_VERSION_PATCH 0)
set(NVIM_VERSION_PRERELEASE "-dev") # for package maintainers

# API level
set(NVIM_API_LEVEL 15)        # Bump this after any API/stdlib change.
set(NVIM_API_LEVEL_COMPAT 0)  # Adjust this after a _breaking_ API change.
set(NVIM_API_PRERELEASE true)

option(ENABLE_ASAN_UBSAN "Enable Clang address & undefined behavior sanitizer for nvim binary." OFF)
option(ENABLE_MSAN "Enable Clang memory sanitizer for nvim binary." OFF)
# TSAN exists to test Luv threads.
option(ENABLE_TSAN "Enable Clang thread sanitizer for nvim binary." OFF)

if((ENABLE_ASAN_UBSAN AND ENABLE_MSAN)
    OR (ENABLE_ASAN_UBSAN AND ENABLE_TSAN)
    OR (ENABLE_MSAN AND ENABLE_TSAN))
  message(FATAL_ERROR "Sanitizers cannot be enabled simultaneously")
endif()

# Place targets in bin/ or lib/ for all build configurations
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)
set(CMAKE_LIBRARY_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/lib)
set(CMAKE_ARCHIVE_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/lib)
foreach(CFGNAME ${CMAKE_CONFIGURATION_TYPES})
  string(TOUPPER ${CFGNAME} CFGNAME)
  set(CMAKE_RUNTIME_OUTPUT_DIRECTORY_${CFGNAME} ${CMAKE_BINARY_DIR}/bin)
  set(CMAKE_LIBRARY_OUTPUT_DIRECTORY_${CFGNAME} ${CMAKE_BINARY_DIR}/lib)
  set(CMAKE_ARCHIVE_OUTPUT_DIRECTORY_${CFGNAME} ${CMAKE_BINARY_DIR}/lib)
endforeach()

if(NOT PREFER_LUA)
  find_program(LUA_PRG NAMES luajit)
endif()
find_program(LUA_PRG NAMES lua5.1 lua5.2 lua)
mark_as_advanced(LUA_PRG)
if(NOT LUA_PRG)
  message(FATAL_ERROR "Failed to find a Lua 5.1-compatible interpreter")
endif()
message(STATUS "Using Lua interpreter: ${LUA_PRG}")

# Lint
option(CI_LINT "Abort if lint programs not found" OFF)
if(CI_LINT)
  set(LINT_REQUIRED "REQUIRED")
endif()
find_program(SHELLCHECK_PRG shellcheck ${LINT_REQUIRED})
mark_as_advanced(SHELLCHECK_PRG)
find_program(STYLUA_PRG stylua ${LINT_REQUIRED})
mark_as_advanced(STYLUA_PRG)
find_program(TS_QUERY_LS_PRG ts_query_ls ${LINT_REQUIRED})
mark_as_advanced(TS_QUERY_LS_PRG)

set(STYLUA_DIRS runtime scripts src test contrib)

add_glob_target(
  TARGET lintlua-stylua
  COMMAND ${STYLUA_PRG}
  FLAGS --color=always --check --respect-ignores
  GLOB_DIRS ${STYLUA_DIRS}
  GLOB_PAT *.lua
  TOUCH_STRATEGY PER_DIR)
# Special handling of some files (which are ignored in .styluaignore).
# Workaround because stylua doesn't(?) support file-specific settings.
add_custom_target(lintlua-stylua2
  COMMAND ${STYLUA_PRG} --config-path "${PROJECT_SOURCE_DIR}/.stylua2.toml"
    --color=always --check
    "${PROJECT_SOURCE_DIR}/test/functional/ui/decorations_spec.lua"
    "${PROJECT_SOURCE_DIR}/test/functional/ui/float_spec.lua"
    "${PROJECT_SOURCE_DIR}/test/functional/ui/multigrid_spec.lua"
)
add_dependencies(lintlua-stylua lintlua-stylua2)

add_custom_target(lintlua)
add_dependencies(lintlua lintlua-stylua)

add_glob_target(
  TARGET lintsh
  COMMAND ${SHELLCHECK_PRG}
  FLAGS -x -a
  GLOB_DIRS scripts
  GLOB_PAT *.sh
  TOUCH_STRATEGY PER_DIR)

add_custom_target(lintquery-compilation
  COMMAND ${TS_QUERY_LS_PRG}
  check ${PROJECT_SOURCE_DIR}/runtime/queries --config
  '{"parser_install_directories": ["${DEPS_PREFIX}/lib/nvim/parser"]}')

add_custom_target(lintquery-format
  COMMAND ${TS_QUERY_LS_PRG}
  format ${PROJECT_SOURCE_DIR}/runtime/queries --check)

add_custom_target(lintquery)
add_dependencies(lintquery lintquery-compilation lintquery-format)

add_custom_target(lintcommit
  COMMAND "${NVIM_BIN_FILE}" --clean -l ${PROJECT_SOURCE_DIR}/scripts/lintcommit.lua main)
add_dependencies(lintcommit nvim_bin)

add_custom_target(linterrcodes
  COMMAND "${NVIM_BIN_FILE}" --clean -l ${PROJECT_SOURCE_DIR}/scripts/linterrcodes.lua
  WORKING_DIRECTORY ${PROJECT_SOURCE_DIR})
add_dependencies(linterrcodes nvim_bin)

add_custom_target(lint)
add_dependencies(lint lintrust lintlua lintsh lintquery)

# Format
add_glob_target(
  TARGET formatlua
  COMMAND ${STYLUA_PRG}
  FLAGS --respect-ignores
  GLOB_DIRS ${STYLUA_DIRS}
  GLOB_PAT *.lua
  TOUCH_STRATEGY PER_DIR)
# Special handling of some files (which are ignored in .styluaignore).
# Workaround because stylua doesn't(?) support file-specific settings.
add_custom_target(formatlua2
  COMMAND ${STYLUA_PRG} --config-path "${PROJECT_SOURCE_DIR}/.stylua2.toml"
    "${PROJECT_SOURCE_DIR}/test/functional/ui/decorations_spec.lua"
    "${PROJECT_SOURCE_DIR}/test/functional/ui/float_spec.lua"
    "${PROJECT_SOURCE_DIR}/test/functional/ui/multigrid_spec.lua"
)
add_dependencies(formatlua formatlua2)

add_custom_target(formatquery
  COMMAND ${TS_QUERY_LS_PRG}
  format ${PROJECT_SOURCE_DIR}/runtime/queries)

add_custom_target(format)
add_dependencies(format formatrust formatlua formatquery)

install_helper(
  FILES ${CMAKE_SOURCE_DIR}/src/man/nvim.1
  DESTINATION ${CMAKE_INSTALL_MANDIR}/man1)

add_custom_target(nvim ALL)
add_dependencies(nvim nvim_bin nvim_runtime_deps nvim_runtime)

add_subdirectory(src/nvim)
add_subdirectory(src/tee)
add_subdirectory(src/xxd)
add_subdirectory(runtime)
add_subdirectory(test)

add_custom_target(uninstall
  COMMAND ${CMAKE_COMMAND} -P ${PROJECT_SOURCE_DIR}/cmake/UninstallHelper.cmake)

if(${CMAKE_SOURCE_DIR} STREQUAL ${CMAKE_CURRENT_SOURCE_DIR})
  add_subdirectory(cmake.packaging)
endif()

if (CMAKE_HOST_SYSTEM_PROCESSOR MATCHES "arm|aarch|ARM")
  set(EMMYLUA_ARCH arm64)
else()
  set(EMMYLUA_ARCH x64)
endif()

string(TOLOWER ${CMAKE_HOST_SYSTEM_NAME} EMMYLUA_SYSTEM)
set(EMMYLUA_ARCHIVE_EXT tar.gz)
set(EMMYLUA_EXE_EXT "")
if(CMAKE_HOST_SYSTEM_NAME STREQUAL "Linux")
  if(EMMYLUA_ARCH STREQUAL "arm64")
    set(EMMYLUA_ARCH aarch64)
  endif()
  string(APPEND EMMYLUA_ARCH "-glibc.2.17")
elseif(CMAKE_HOST_WIN32)
  set(EMMYLUA_SYSTEM win32)
  set(EMMYLUA_ARCHIVE_EXT zip)
  set(EMMYLUA_EXE_EXT .exe)
endif()

set(EMMYLUA_VERSION 0.25.1)
set(EMMYLUA_ARCHIVE "emmylua_check-${EMMYLUA_SYSTEM}-${EMMYLUA_ARCH}.${EMMYLUA_ARCHIVE_EXT}")
set(EMMYLUA_URL https://github.com/EmmyLuaLs/emmylua-analyzer-rust/releases/download/${EMMYLUA_VERSION}/${EMMYLUA_ARCHIVE})

ExternalProject_Add(download_emmylua
  URL ${EMMYLUA_URL}
  DOWNLOAD_DIR ${DEPS_DOWNLOAD_DIR}/emmylua
  SOURCE_DIR ${DEPS_BIN_DIR}/emmylua
  CONFIGURE_COMMAND ""
  BUILD_COMMAND ""
  INSTALL_COMMAND ""
  EXCLUDE_FROM_ALL TRUE
  DOWNLOAD_NO_PROGRESS TRUE
  CMAKE_CACHE_ARGS ${DEPS_CMAKE_CACHE_ARGS})

add_custom_target(emmylua-check
  COMMAND ${DEPS_BIN_DIR}/emmylua/emmylua_check${EMMYLUA_EXE_EXT}
    ${PROJECT_SOURCE_DIR}/runtime
    --config ${PROJECT_SOURCE_DIR}/.emmyrc.json
    --warnings-as-errors
  WORKING_DIRECTORY ${PROJECT_SOURCE_DIR}
  USES_TERMINAL
  VERBATIM)

add_dependencies(emmylua-check download_emmylua)
