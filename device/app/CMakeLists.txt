# © 2025 Unit Circle Inc

cmake_minimum_required(VERSION 3.20.0)



# west build -S uclog-target1
# Or we can force
# See: https://docs.zephyrproject.org/latest/build/snippets/using.html#application-required-snippets
if(NOT uclog-target1 IN_LIST SNIPPET)
  set(SNIPPET uclog-target1 ${SNIPPET} CACHE STRING "" FORCE)
endif()
if(NOT uclog-shell IN_LIST SNIPPET)
  set(SNIPPET uclog-shell ${SNIPPET} CACHE STRING "" FORCE)
endif()


find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})

zephyr_compile_options( -Werror )

project(app)

target_sources(app PRIVATE src/main.c
)
