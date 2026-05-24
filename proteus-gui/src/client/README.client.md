# Local JSON helper — placeholder for `proteus-client`

`ProteusRunner` and `PkexecRunner` are **skeleton placeholders**.

Once the `proteus-client` shared C++ library unit has merged, replace:

| This file | With |
|---|---|
| `ProteusRunner.h` / `.cpp` | `#include <proteus-client/Reader.h>` wrapper |
| `PkexecRunner.h` / `.cpp` | `#include <proteus-client/ElevatedRunner.h>` wrapper |

The public API of both classes must remain identical so no page (StatusPage,
PersonaPage, SsidPage, DoctorPage) needs to change.

`proteus-client` will live in `proteus-client/` at the repo root.  Its
CMake target name is `proteus::client`.  Add it to `target_link_libraries`
in `CMakeLists.txt` and remove the local source files from `GUI_SOURCES`.
