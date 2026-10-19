if(CMAKE_BUILD_TYPE STREQUAL "Debug" OR NOT CMAKE_BUILD_TYPE)
  set(XXD_RUST_PROFILE_DIR "debug")
else()
  set(XXD_RUST_PROFILE_DIR "release")
endif()

set(XXD_CARGO_BIN "${PROJECT_BINARY_DIR}/cargo/${XXD_RUST_PROFILE_DIR}/xxd${CMAKE_EXECUTABLE_SUFFIX}")
set(XXD_OUT_BIN "${CMAKE_BINARY_DIR}/bin/xxd${CMAKE_EXECUTABLE_SUFFIX}")

add_custom_command(
  OUTPUT "${XXD_OUT_BIN}"
  COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/bin"
  COMMAND ${CMAKE_COMMAND} -E copy "${XXD_CARGO_BIN}" "${XXD_OUT_BIN}"
  DEPENDS nvim_rs_build "${PROJECT_SOURCE_DIR}/src/xxd/main.rs")

add_custom_target(xxd ALL DEPENDS "${XXD_OUT_BIN}")

if(WIN32)
  install(PROGRAMS "${XXD_OUT_BIN}"
    DESTINATION "${CMAKE_INSTALL_BINDIR}"
  )
endif()
