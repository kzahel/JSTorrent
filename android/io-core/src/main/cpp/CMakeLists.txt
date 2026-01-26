cmake_minimum_required(VERSION 3.22.1)
project(io-native)

# Mmap helper for wrapping native addresses in DirectByteBuffer
add_library(io-native SHARED
    mmap-helper.c
)

# Link against Android log library
find_library(log-lib log)
target_link_libraries(io-native ${log-lib})

# Compiler options
target_compile_options(io-native PRIVATE
    -O2
    -fvisibility=hidden
    -fPIC
    -Wall
)

set_target_properties(io-native PROPERTIES
    C_STANDARD 11
    C_STANDARD_REQUIRED ON
)
