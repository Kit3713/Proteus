# SPDX-License-Identifier: GPL-3.0-or-later
#
# proteus-client — C++ Qt6 library that wraps the `proteus` CLI.
# Provides read (QProcess + --json) and mutate (pkexec + --yes) helpers,
# plus a journald-tail stub for live event streaming.
#
# Qt6 is required at build time.  On Fedora 43+:
#   sudo dnf install qt6-qtbase-devel cmake gcc-c++
#
# The shared library version tracks the proteus CLI version it targets
# (see README.md).  The Fedora sub-package declares:
#   Requires: proteus = %{version}
# so the CLI and this library are always upgraded together.

cmake_minimum_required(VERSION 3.21)

project(
    proteus-client
    VERSION 1.3.0
    DESCRIPTION "Qt6 wrapper library around the proteus CLI (read + mutate + events)"
    LANGUAGES CXX
)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# ── Qt6 ───────────────────────────────────────────────────────────────────────
# On the CI / build host this find_package will fail when Qt6 is absent —
# that is expected and documented.  See README.md.
find_package(Qt6 REQUIRED COMPONENTS Core Concurrent)
qt_standard_project_setup()

# ── Library sources ───────────────────────────────────────────────────────────
set(PROTEUS_CLIENT_SOURCES
    src/ProteusRunner.cpp
    src/ProteusMutate.cpp
    src/JournaldTail.cpp
)

set(PROTEUS_CLIENT_HEADERS
    src/ProteusRunner.h
    src/ProteusMutate.h
    src/JournaldTail.h
)

add_library(proteus-client STATIC
    ${PROTEUS_CLIENT_SOURCES}
    ${PROTEUS_CLIENT_HEADERS}
)

target_include_directories(proteus-client
    PUBLIC
        $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/src>
        $<INSTALL_INTERFACE:include/proteus-client>
)

target_link_libraries(proteus-client
    PUBLIC
        Qt6::Core
        Qt6::Concurrent
)

# ── Compiler hardening ────────────────────────────────────────────────────────
target_compile_options(proteus-client PRIVATE
    -Wall -Wextra -Wpedantic
    $<$<CONFIG:Release>:-O2>
)

# ── Installation ──────────────────────────────────────────────────────────────
include(GNUInstallDirs)

install(TARGETS proteus-client
    EXPORT proteus-client-targets
    ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR}
    LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}
)

install(FILES ${PROTEUS_CLIENT_HEADERS}
    DESTINATION ${CMAKE_INSTALL_INCLUDEDIR}/proteus-client
)
