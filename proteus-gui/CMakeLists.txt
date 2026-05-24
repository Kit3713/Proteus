# SPDX-License-Identifier: GPL-3.0-or-later
#
# proteus-gui — Qt6 Widgets shell (Kirigami optional).
#
# Architecture contract
# ---------------------
# This binary is a THIN CLIENT over the proteus CLI.  It never links against
# any Rust or proteus library code.  All reads are performed by spawning
# `proteus … --json` via QProcess; all mutations go through
# `pkexec proteus … --yes`.  Qt/KDE dependencies must NEVER enter the Rust
# binary (and they don't — the GUI is a separate executable).
#
# Build deps (Fedora 43+):
#   sudo dnf install qt6-qtbase-devel kf6-kirigami-devel cmake gcc-c++
#   (Kirigami is optional; the build falls back to a plain Qt Widgets shell
#   when kf6-kirigami-devel is absent — see the find_package block below.)
#
# Minimum versions
#   CMake  3.16   (target_sources / target_include_directories modern form)
#   Qt     6.2    (LTS baseline shipped in Fedora 38+)
#   C++    17     (structured bindings, std::optional, if-init, …)
#
# Version-pin policy (see README.md §"Package version pinning"):
#   The RPM spec (dist/rpm/proteus-gui.spec, not created yet) will declare:
#     Requires: proteus-tray = %{version}
#   so the GUI, tray, and CLI all move in lock-step.  Do NOT relax that pin
#   without explicit review — the JSON shapes the CLI emits are not versioned
#   independently of the binary.

cmake_minimum_required(VERSION 3.16)

project(
    proteus-gui
    VERSION 0.1.0
    DESCRIPTION "Proteus desktop GUI — thin Qt client over the proteus CLI"
    LANGUAGES CXX
)

# ── C++ standard ──────────────────────────────────────────────────────────────
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)          # -std=c++17, not -std=gnu++17

# ── Qt6 (required) ────────────────────────────────────────────────────────────
# Qt6 is absent on the CI / snapshot host used to author this skeleton.
# CMake will correctly abort here with "Could not find a package configuration
# file provided by Qt6" — that is the ONLY expected failure.  The source tree
# is structurally complete and ready to compile on a machine that has the Qt6
# development packages installed.
find_package(Qt6 REQUIRED COMPONENTS Core Gui Widgets)

qt_standard_project_setup()

# ── Kirigami (optional) ────────────────────────────────────────────────────────
# When kf6-kirigami-devel is present we compile with Kirigami and get
# proper navigation cards, page stacks, etc.  When it's absent we fall
# through to a plain QStackedWidget + QListWidget sidebar that mirrors the
# same page topology.
find_package(KF6Kirigami QUIET)
if(KF6Kirigami_FOUND)
    message(STATUS "proteus-gui: Kirigami found — building Kirigami shell")
    set(HAVE_KIRIGAMI TRUE)
else()
    message(STATUS "proteus-gui: Kirigami NOT found — building Qt Widgets shell (fully supported)")
    set(HAVE_KIRIGAMI FALSE)
endif()

# ── Sources ───────────────────────────────────────────────────────────────────
set(GUI_SOURCES
    src/main.cpp
    src/MainWindow.cpp
    src/MainWindow.h

    # Local JSON/process helper
    # TODO: replace with proteus-client (shared lib) once that unit has merged.
    # See src/client/README.client.md for the replacement contract.
    src/client/ProteusRunner.cpp
    src/client/ProteusRunner.h
    src/client/PkexecRunner.cpp
    src/client/PkexecRunner.h

    # Pages
    src/pages/StatusPage.cpp
    src/pages/StatusPage.h
    src/pages/PersonaPage.cpp
    src/pages/PersonaPage.h
    src/pages/SsidPage.cpp
    src/pages/SsidPage.h
    src/pages/DoctorPage.cpp
    src/pages/DoctorPage.h
)

qt_add_executable(proteus-gui ${GUI_SOURCES})

# ── Compile definitions ───────────────────────────────────────────────────────
target_compile_definitions(proteus-gui PRIVATE
    PROTEUS_GUI_VERSION="${PROJECT_VERSION}"
    $<$<BOOL:${HAVE_KIRIGAMI}>:HAVE_KIRIGAMI>
)

# ── Include paths ─────────────────────────────────────────────────────────────
target_include_directories(proteus-gui PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src
    ${CMAKE_CURRENT_SOURCE_DIR}/src/client
    ${CMAKE_CURRENT_SOURCE_DIR}/src/pages
)

# ── Link libraries ────────────────────────────────────────────────────────────
target_link_libraries(proteus-gui PRIVATE
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
)

if(HAVE_KIRIGAMI)
    target_link_libraries(proteus-gui PRIVATE KF6::Kirigami)
endif()

# ── Install rules (future) ────────────────────────────────────────────────────
# TODO: when the RPM/Flatpak packaging is wired up, add:
#   install(TARGETS proteus-gui RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
#   install(FILES resources/proteus-gui.desktop DESTINATION share/applications)
#   install(FILES resources/proteus.svg         DESTINATION share/icons/hicolor/scalable/apps)
