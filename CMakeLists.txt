# © 2025 Unit Circle Inc

set(APP_ROOT_DIR "${CMAKE_CURRENT_SOURCE_DIR}")

zephyr_include_directories(include)
add_subdirectory(drivers)
add_subdirectory(lib)
