option(PREFER_LUA "Prefer Lua over LuaJIT in the nvim executable." OFF)
option(CI_BUILD "CI, extra flags will be set" OFF)
if(CI_BUILD)
  message(STATUS "CI build enabled")
endif()

#-------------------------------------------------------------------------------
# Variables
#-------------------------------------------------------------------------------

set(BINARY_LIB_DIR ${PROJECT_BINARY_DIR}/lib/nvim)
set(GENERATOR_DIR ${PROJECT_SOURCE_DIR}/src/gen)
set(GEN_EVAL_TOUCH ${TOUCHES_DIR}/gen_doc_eval)
set(LUAJIT_RUNTIME_DIR ${DEPS_PREFIX}/share/luajit-2.1/jit)
set(NVIM_RUNTIME_DIR ${PROJECT_SOURCE_DIR}/runtime)
set(NVIM_LUA_PRELOAD ${GENERATOR_DIR}/preload.lua)

file(GLOB API_HEADERS CONFIGURE_DEPENDS api/*.rs)

file(MAKE_DIRECTORY ${TOUCHES_DIR})

set(NVIM_VERSION
    "v${NVIM_VERSION_MAJOR}.${NVIM_VERSION_MINOR}.${NVIM_VERSION_PATCH}${NVIM_VERSION_PRERELEASE}")
set(NVIM_VERSION_BUILD "")
execute_process(
  COMMAND git --git-dir=${PROJECT_SOURCE_DIR}/.git --work-tree=${PROJECT_SOURCE_DIR} describe --first-parent --dirty --always
  OUTPUT_VARIABLE GIT_TAG
  OUTPUT_STRIP_TRAILING_WHITESPACE
  ERROR_QUIET
  RESULT_VARIABLE RES)
if(NOT RES)
  string(REGEX REPLACE ".*\\-" "" NVIM_VERSION_BUILD "${GIT_TAG}")
  if(NVIM_VERSION_PRERELEASE)
    string(REGEX REPLACE "^v[0-9]+.[0-9]+.[0-9]+-" "" NVIM_VERSION_GIT "${GIT_TAG}")
    string(REGEX REPLACE "^([0-9]+)-([a-z0-9]+)" "\\1+\\2" NVIM_VERSION_GIT "${NVIM_VERSION_GIT}")
    set(NVIM_VERSION "${NVIM_VERSION}-${NVIM_VERSION_GIT}")
  endif()
endif()
message(STATUS "Using NVIM_VERSION: ${NVIM_VERSION}")

set(NVIM_LUA "${NVIM_BIN_FILE}" -u NONE -l ${NVIM_LUA_PRELOAD} ${PROJECT_SOURCE_DIR})
set(NVIM_LUA_DEPS nvim_bin nvim_runtime_deps ${NVIM_LUA_PRELOAD})

if(PREFER_LUA)
  message(STATUS "luajit not used, skipping unit tests")
endif()

find_program(CARGO_PRG cargo
  HINTS "$ENV{HOME}/.cargo/bin" "$ENV{USERPROFILE}/.cargo/bin"
  REQUIRED)

set(NVIM_RUST_TARGET_DIR "${PROJECT_BINARY_DIR}/cargo")
if(CMAKE_BUILD_TYPE STREQUAL "Debug" OR NOT CMAKE_BUILD_TYPE)
  set(NVIM_RUST_PROFILE_DIR "debug")
  set(NVIM_RUST_CARGO_FLAGS "")
else()
  set(NVIM_RUST_PROFILE_DIR "release")
  set(NVIM_RUST_CARGO_FLAGS "--release")
endif()
if(PREFER_LUA)
  list(APPEND NVIM_RUST_CARGO_FLAGS "--features" "vendor-bit")
endif()

set(NVIM_CARGO_BIN
  "${NVIM_RUST_TARGET_DIR}/${NVIM_RUST_PROFILE_DIR}/nvim${CMAKE_EXECUTABLE_SUFFIX}")

file(GLOB_RECURSE NVIM_RUST_SOURCES CONFIGURE_DEPENDS
  "${CMAKE_CURRENT_LIST_DIR}/*.rs"
  "${NVIM_RUNTIME_DIR}/lua/vim/*.lua"
  "${NVIM_RUNTIME_DIR}/lua/vim/_core/*.lua"
  "${PROJECT_SOURCE_DIR}/build.rs"
  "${PROJECT_SOURCE_DIR}/src/bit.rs"
  "${PROJECT_SOURCE_DIR}/src/cjson/*.rs"
  "${PROJECT_SOURCE_DIR}/src/lpeg/*.rs"
  "${PROJECT_SOURCE_DIR}/src/mpack/*.rs"
  "${PROJECT_SOURCE_DIR}/src/tee/*.rs"
  "${PROJECT_SOURCE_DIR}/src/xdiff/*.rs"
  "${PROJECT_SOURCE_DIR}/src/xxd/*.rs"
  "${PROJECT_SOURCE_DIR}/test/functional/fixtures/*.rs")

if(ENABLE_ASAN_UBSAN OR ENABLE_MSAN OR ENABLE_TSAN)
  set(NVIM_RUST_EXITFREE "1")
else()
  set(NVIM_RUST_EXITFREE "0")
endif()
if(ENABLE_UNIBILIUM)
  set(NVIM_RUST_UNIBILIUM "1")
else()
  set(NVIM_RUST_UNIBILIUM "0")
endif()

add_custom_command(
  OUTPUT "${NVIM_CARGO_BIN}"
  BYPRODUCTS
    "${NVIM_RUST_TARGET_DIR}/${NVIM_RUST_PROFILE_DIR}/tee${CMAKE_EXECUTABLE_SUFFIX}"
    "${NVIM_RUST_TARGET_DIR}/${NVIM_RUST_PROFILE_DIR}/xxd${CMAKE_EXECUTABLE_SUFFIX}"
    "${NVIM_RUST_TARGET_DIR}/${NVIM_RUST_PROFILE_DIR}/shell-test${CMAKE_EXECUTABLE_SUFFIX}"
    "${NVIM_RUST_TARGET_DIR}/${NVIM_RUST_PROFILE_DIR}/printargs-test${CMAKE_EXECUTABLE_SUFFIX}"
    "${NVIM_RUST_TARGET_DIR}/${NVIM_RUST_PROFILE_DIR}/printenv-test${CMAKE_EXECUTABLE_SUFFIX}"
    "${NVIM_RUST_TARGET_DIR}/${NVIM_RUST_PROFILE_DIR}/streams-test${CMAKE_EXECUTABLE_SUFFIX}"
    "${NVIM_RUST_TARGET_DIR}/${NVIM_RUST_PROFILE_DIR}/tty-test${CMAKE_EXECUTABLE_SUFFIX}"
  COMMAND ${CMAKE_COMMAND} -E env
    "CARGO_TARGET_DIR=${NVIM_RUST_TARGET_DIR}"
    "DEPS_PREFIX=${DEPS_PREFIX}"
    "VTERM_TEST_FILE=${VTERM_TEST_FILE}"
    "NVIM_VERSION_LONG=NVIM ${NVIM_VERSION}"
    "NVIM_VERSION_BUILD=${NVIM_VERSION_BUILD}"
    "NVIM_BUILD_TYPE=Build type: $<CONFIG>"
    "NVIM_VERSION_CFLAGS=$<$<NOT:$<OR:$<CONFIG:Release>,$<CONFIG:MinSizeRel>>>:Compilation: ${CARGO_PRG} -DHAVE_UNIBILIUM=${NVIM_RUST_UNIBILIUM}>"
    "NVIM_DEFAULT_VIM_DIR=${CMAKE_INSTALL_FULL_DATAROOTDIR}/nvim"
    "NVIM_DEFAULT_LIB_DIR=${CMAKE_INSTALL_FULL_LIBDIR}/nvim"
    "NVIM_EXITFREE=${NVIM_RUST_EXITFREE}"
    "NVIM_HAVE_UNIBILIUM=${NVIM_RUST_UNIBILIUM}"
    "${CARGO_PRG}" build
    --manifest-path "${PROJECT_SOURCE_DIR}/Cargo.toml"
    ${NVIM_RUST_CARGO_FLAGS}
  DEPENDS
    "${PROJECT_SOURCE_DIR}/Cargo.toml"
    ${NVIM_RUST_SOURCES}
  WORKING_DIRECTORY "${PROJECT_SOURCE_DIR}"
  USES_TERMINAL)

add_custom_target(nvim_rs_build DEPENDS "${NVIM_CARGO_BIN}")

add_custom_command(
  OUTPUT "${NVIM_BIN_FILE}"
  COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/bin"
  COMMAND ${CMAKE_COMMAND} -E copy "${NVIM_CARGO_BIN}" "${NVIM_BIN_FILE}"
  DEPENDS "${NVIM_CARGO_BIN}" nvim_rs_build)

add_custom_target(nvim_bin DEPENDS "${NVIM_BIN_FILE}")

add_custom_target(testrust
  COMMAND ${CMAKE_COMMAND} -E env
    "CARGO_TARGET_DIR=${NVIM_RUST_TARGET_DIR}"
    "DEPS_PREFIX=${DEPS_PREFIX}"
    "VTERM_TEST_FILE=${VTERM_TEST_FILE}"
    "${CARGO_PRG}" test
    --manifest-path "${PROJECT_SOURCE_DIR}/Cargo.toml"
  WORKING_DIRECTORY "${PROJECT_SOURCE_DIR}"
  USES_TERMINAL)

install(PROGRAMS "${NVIM_BIN_FILE}" DESTINATION ${CMAKE_INSTALL_BINDIR})
if(MSVC)
  install(FILES "${NVIM_RUST_TARGET_DIR}/${NVIM_RUST_PROFILE_DIR}/nvim.pdb" DESTINATION ${CMAKE_INSTALL_BINDIR} OPTIONAL)
endif()

add_custom_target(nvim_runtime_deps)
if(WIN32)
  # Copy DLLs and third-party tools to bin/ and install them along with nvim
  add_custom_command(TARGET nvim_runtime_deps
    POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E ${COPY_DIRECTORY} ${PROJECT_BINARY_DIR}/windows_runtime_deps/
      ${CMAKE_RUNTIME_OUTPUT_DIRECTORY})
  install(DIRECTORY ${PROJECT_BINARY_DIR}/windows_runtime_deps/
    DESTINATION ${CMAKE_INSTALL_BINDIR})

  add_custom_target(nvim_dll_deps DEPENDS nvim_bin
    COMMAND ${CMAKE_COMMAND} -E make_directory ${PROJECT_BINARY_DIR}/windows_runtime_deps
    COMMAND ${CMAKE_COMMAND}
      -D CMAKE_PREFIX_PATH=${CMAKE_PREFIX_PATH}
      -D BINARY="${NVIM_BIN_FILE}"
      -D DST=${PROJECT_BINARY_DIR}/windows_runtime_deps
      -D CI_BUILD=${CI_BUILD}
      -P ${PROJECT_SOURCE_DIR}/cmake/WindowsDllCopy.cmake)
  add_dependencies(nvim_runtime_deps nvim_dll_deps)

  set(EXTERNAL_BLOBS_SCRIPT
    "file(MAKE_DIRECTORY \"${PROJECT_BINARY_DIR}/windows_runtime_deps/platforms\")")
  foreach(DEP_FILE IN ITEMS
      win32yank.exe
  )
  get_filename_component(DEP_FILE_DIR ${DEP_FILE} DIRECTORY)
  set(EXTERNAL_BLOBS_SCRIPT "${EXTERNAL_BLOBS_SCRIPT}\n"
    "file(COPY \"${DEPS_PREFIX}/bin/${DEP_FILE}\"
    DESTINATION \"${PROJECT_BINARY_DIR}/windows_runtime_deps/${DEP_FILE_DIR}\")")
  endforeach()
  file(WRITE ${PROJECT_BINARY_DIR}/external_blobs.cmake ${EXTERNAL_BLOBS_SCRIPT})
  add_custom_target(external_blobs
    COMMAND ${CMAKE_COMMAND} -P ${PROJECT_BINARY_DIR}/external_blobs.cmake)
  add_dependencies(nvim_runtime_deps external_blobs)
endif()

file(MAKE_DIRECTORY ${BINARY_LIB_DIR})

# install treesitter parser if bundled
if(EXISTS ${DEPS_PREFIX}/lib/nvim/parser)
  add_custom_command(
    TARGET nvim_runtime_deps
    POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E ${COPY_DIRECTORY} ${DEPS_PREFIX}/lib/nvim/parser ${BINARY_LIB_DIR}/parser)
endif()

if(CYGWIN)
  if(NOT PREFER_LUA)
    add_custom_command(
      TARGET nvim_runtime_deps
      POST_BUILD
      COMMAND ${CMAKE_COMMAND} -E copy ${DEPS_PREFIX}/bin/cyglua51.dll ${PROJECT_BINARY_DIR}/bin)
    install(PROGRAMS ${PROJECT_BINARY_DIR}/bin/cyglua51.dll DESTINATION ${CMAKE_INSTALL_BINDIR})
  endif()
endif()

install(DIRECTORY ${BINARY_LIB_DIR}
  DESTINATION ${CMAKE_INSTALL_LIBDIR}
  USE_SOURCE_PERMISSIONS)

if(NOT PREFER_LUA)
  # install luajit runtime files if bundled
  if(EXISTS ${LUAJIT_RUNTIME_DIR})
    install(DIRECTORY ${LUAJIT_RUNTIME_DIR}
      DESTINATION ${CMAKE_INSTALL_DATAROOTDIR}/nvim/runtime/lua
      USE_SOURCE_PERMISSIONS)
  endif()
endif()

#-------------------------------------------------------------------------------
# Lint
#-------------------------------------------------------------------------------

add_custom_target(lintrust
  COMMAND ${CMAKE_COMMAND} -E env
    "CARGO_TARGET_DIR=${NVIM_RUST_TARGET_DIR}"
    "DEPS_PREFIX=${DEPS_PREFIX}"
    "VTERM_TEST_FILE=${VTERM_TEST_FILE}"
    "${CARGO_PRG}" clippy
    --manifest-path "${PROJECT_SOURCE_DIR}/Cargo.toml"
    --all-targets -- -D warnings
  WORKING_DIRECTORY "${PROJECT_SOURCE_DIR}"
  USES_TERMINAL)

add_custom_target(formatrust
  COMMAND "${CARGO_PRG}" fmt
    --manifest-path "${PROJECT_SOURCE_DIR}/Cargo.toml"
  WORKING_DIRECTORY "${PROJECT_SOURCE_DIR}"
  USES_TERMINAL)

#-------------------------------------------------------------------------------
# Docs
#-------------------------------------------------------------------------------

add_subdirectory(po)

file(GLOB LUA_SOURCES CONFIGURE_DEPENDS
  ${NVIM_RUNTIME_DIR}/lua/vim/*.lua
  ${NVIM_RUNTIME_DIR}/lua/vim/async/*.lua
  ${NVIM_RUNTIME_DIR}/lua/vim/_meta/*.lua
  ${NVIM_RUNTIME_DIR}/lua/vim/filetype/*.lua
  ${NVIM_RUNTIME_DIR}/lua/vim/lsp/*.lua
  ${NVIM_RUNTIME_DIR}/lua/vim/treesitter/*.lua
)

add_target(doc-vim
  COMMAND ${NVIM_LUA} src/gen/gen_vimdoc.lua
  DEPENDS
    nvim
    ${API_HEADERS}
    ${LUA_SOURCES}
    ${PROJECT_SOURCE_DIR}/src/gen/gen_vimdoc.lua
    ${NVIM_RUNTIME_DIR}/doc/api.txt
    ${NVIM_RUNTIME_DIR}/doc/diagnostic.txt
    ${NVIM_RUNTIME_DIR}/doc/lsp.txt
    ${NVIM_RUNTIME_DIR}/doc/lua-async.txt
    ${NVIM_RUNTIME_DIR}/doc/lua.txt
    ${NVIM_RUNTIME_DIR}/doc/treesitter.txt
  )

add_target(doc-eval
  COMMAND ${NVIM_LUA} ${PROJECT_SOURCE_DIR}/src/gen/gen_eval_files.lua ${PROJECT_BINARY_DIR}/runtime/doc/tags
  DEPENDS
    nvim
    ${API_HEADERS}
    ${PROJECT_SOURCE_DIR}/src/gen/gen_eval_files.lua
    ${PROJECT_SOURCE_DIR}/src/nvim/eval.lua
    ${PROJECT_SOURCE_DIR}/src/nvim/options.lua
    ${PROJECT_SOURCE_DIR}/src/nvim/vvars.lua
    ${NVIM_RUNTIME_DIR}/doc/vimfn.txt
  )

add_custom_target(doc)
add_dependencies(doc doc-vim doc-eval)

add_target(lintdoc
  COMMAND ${NVIM_LUA} scripts/lintdoc.lua
  DEPENDS ${DOCFILES}
  CUSTOM_COMMAND_ARGS USES_TERMINAL)
add_dependencies(lintdoc nvim)

add_target(lintdocurls
  COMMAND ${NVIM_LUA} scripts/lintdoc.lua true
  DEPENDS ${DOCFILES}
  CUSTOM_COMMAND_ARGS USES_TERMINAL)
add_dependencies(lintdocurls nvim)
