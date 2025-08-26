# © 2025 Unit Circle Inc

cmake_minimum_required(VERSION 3.20.0)

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})

zephyr_compile_options( -Werror )

project(app)

target_sources(app PRIVATE src/main.c
)
