cmake_minimum_required(VERSION 3.20)
project(BouchaudSkiaProbe LANGUAGES CXX)

find_package(unofficial-skia CONFIG REQUIRED)
find_package(Vulkan REQUIRED)

add_executable(skia-vulkan-probe main.cpp)
target_compile_features(skia-vulkan-probe PRIVATE cxx_std_20)
target_link_libraries(skia-vulkan-probe PRIVATE unofficial::skia::skia Vulkan::Vulkan)
