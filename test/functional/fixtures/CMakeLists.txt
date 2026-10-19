add_library(test_lib INTERFACE)
if(WIN32)
  target_compile_definitions(test_lib INTERFACE MSWIN)
endif()
target_link_libraries(test_lib INTERFACE nvim_bin)

if(CMAKE_BUILD_TYPE STREQUAL "Debug" OR NOT CMAKE_BUILD_TYPE)
  set(FIXTURE_RUST_PROFILE_DIR "debug")
else()
  set(FIXTURE_RUST_PROFILE_DIR "release")
endif()

macro(add_rust_fixture_bin target_name cargo_bin_name src_file)
  set(_cargo_bin "${PROJECT_BINARY_DIR}/cargo/${FIXTURE_RUST_PROFILE_DIR}/${cargo_bin_name}${CMAKE_EXECUTABLE_SUFFIX}")
  set(_out_bin "${CMAKE_BINARY_DIR}/bin/${target_name}${CMAKE_EXECUTABLE_SUFFIX}")
  add_custom_command(
    OUTPUT "${_out_bin}"
    COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/bin"
    COMMAND ${CMAKE_COMMAND} -E copy "${_cargo_bin}" "${_out_bin}"
    DEPENDS nvim_rs_build "${CMAKE_CURRENT_LIST_DIR}/${src_file}")
  add_custom_target(${target_name} DEPENDS "${_out_bin}")
endmacro()

add_rust_fixture_bin(shell-test shell-test shell-test.rs)
# Fake pwsh (powershell) for testing make_filter_cmd(). #16271
add_rust_fixture_bin(pwsh-test shell-test shell-test.rs)
add_rust_fixture_bin(printargs-test printargs-test printargs-test.rs)
add_rust_fixture_bin(printenv-test printenv-test printenv-test.rs)

add_executable(tty-test EXCLUDE_FROM_ALL tty-test.c)
add_executable(streams-test EXCLUDE_FROM_ALL streams-test.c)

target_link_libraries(tty-test PRIVATE test_lib)
target_link_libraries(streams-test PRIVATE test_lib)
