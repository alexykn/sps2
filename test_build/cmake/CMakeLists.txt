cmake_minimum_required(VERSION 3.10)
project(hello-cmake VERSION 1.0.0)

set(CMAKE_C_STANDARD 99)

add_executable(hello-cmake hello.c)

install(TARGETS hello-cmake DESTINATION bin)