// SPDX-License-Identifier: GPL-3.0-or-later
//
// main.cpp — entry point for proteus-tray.
//
// Lifecycle:
//   1. Construct QApplication (required for QSystemTrayIcon + Widgets).
//   2. Bail out early if the desktop has no system tray (informative error).
//   3. Construct TrayApp, call init(), enter the event loop.
//
// No privilege is held by this process; all mutating operations are
// dispatched through pkexec (see ProteusMutate).

#include <QApplication>
#include <QMessageBox>
#include <QSystemTrayIcon>

#include "TrayApp.h"

int main(int argc, char *argv[])
{
    // QApplication must be constructed before any widget or icon.
    QApplication app(argc, argv);

    // Prevent the application from exiting when the last window is closed —
    // the tray icon keeps the process alive.
    app.setQuitOnLastWindowClosed(false);

    // Inform the user if the running desktop has no system tray.
    // This can happen on GNOME without the AppIndicator extension, or on
    // bare Wayland compositors without a notification area.
    if (!QSystemTrayIcon::isSystemTrayAvailable()) {
        QMessageBox::critical(
            nullptr,
            QStringLiteral("Proteus Tray"),
            QStringLiteral(
                "No system tray is available on this desktop.\n\n"
                "On GNOME, install the AppIndicator extension and re-launch.\n"
                "On KDE, Xfce, and most other DEs this should work out of the box."
            )
        );
        return 1;
    }

    Proteus::TrayApp trayApp;
    trayApp.init();

    return app.exec();
}
