# SPDX-License-Identifier: GPL-3.0-or-later
#
# proteus-tray — QSystemTrayIcon (StatusNotifierItem) daemon for Proteus.
#
# Thin client over the proteus CLI:
#   READ state:   via proteus-client (QProcess + --json)
#   WRITE state:  via proteus-client (pkexec + --yes)
#   LIVE events:  via proteus-client JournaldTail stub
#
# Desktop integration:
#   - Registers a StatusNotifierItem (KDE / GNOME-with-extension / Xfce)
#     via QSystemTrayIcon; falls back gracefully when no system tray is present.
#   - .desktop autostart file installed to /etc/xdg/autostart/.
#
# Qt6 is required at build time.  On Fedora 43+:
#   sudo dnf install qt6-qtbase-devel qt6-qtwidgets-devel cmake gcc-c++
#
# proteus-client must be built first (or installed as a package).

cmake_minimum_required(VERSION 3.21)

project(
    proteus-tray
    VERSION 1.3.0
    DESCRIPTION "QSystemTrayIcon tray daemon for Proteus (thin CLI front-end)"
    LANGUAGES CXX
)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# ── Qt6 ───────────────────────────────────────────────────────────────────────
# On the CI / build host this find_package will fail when Qt6 is absent —
# that is expected and documented.  See README.md + unit PR description.
find_package(Qt6 REQUIRED COMPONENTS Core Widgets)
qt_standard_project_setup()

# ── Locate proteus-client ────────────────────────────────────────────────────
# When building from source as a sub-project, add proteus-client first:
#   cmake -S . -B build -DCMAKE_SOURCE_DIR=<monorepo-root>
# When installed as a package, find it via CMake config or pkg-config.
#
# We prefer an add_subdirectory approach for in-tree development.
if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/../proteus-client/CMakeLists.txt")
    add_subdirectory(
        "${CMAKE_CURRENT_SOURCE_DIR}/../proteus-client"
        "${CMAKE_CURRENT_BINARY_DIR}/proteus-client"
    )
else()
    # TODO(roadmap-1.3.x): generate and install proteus-clientConfig.cmake
    # so this find_package works for installed packages.
    find_package(proteus-client REQUIRED)
endif()

# ── Sources ───────────────────────────────────────────────────────────────────
set(TRAY_SOURCES
    src/main.cpp
    src/TrayApp.cpp
    src/PopupWindow.cpp
)

set(TRAY_HEADERS
    src/TrayApp.h
    src/PopupWindow.h
)

add_executable(proteus-tray
    ${TRAY_SOURCES}
    ${TRAY_HEADERS}
)

target_link_libraries(proteus-tray
    PRIVATE
        proteus-client
        Qt6::Core
        Qt6::Widgets
)

target_compile_options(proteus-tray PRIVATE
    -Wall -Wextra -Wpedantic
    $<$<CONFIG:Release>:-O2>
)

# ── Desktop integration ───────────────────────────────────────────────────────
install(TARGETS proteus-tray
    RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR}
)

# XDG autostart: launch the tray when the desktop session starts.
install(FILES
    data/proteus-tray.desktop
    DESTINATION ${CMAKE_INSTALL_SYSCONFDIR}/xdg/autostart
)
