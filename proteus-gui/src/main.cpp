// SPDX-License-Identifier: GPL-3.0-or-later
//
// proteus-gui — entry point.
//
// The GUI is a THIN CLIENT over the proteus CLI.  It never links against
// any Rust library code.  Reads via `proteus … --json`; mutations via
// `pkexec proteus … --yes`.
//
// Single-instance policy: TODO (1.4.1) — use a QLocalServer lock file so
// clicking the .desktop file a second time raises the existing window instead
// of spawning a duplicate.

#include "MainWindow.h"

#include <QApplication>
#include <QIcon>

int main(int argc, char *argv[])
{
    QApplication app(argc, argv);

    app.setApplicationName(QStringLiteral("proteus-gui"));
    app.setApplicationDisplayName(QStringLiteral("Proteus"));
    app.setApplicationVersion(QStringLiteral(PROTEUS_GUI_VERSION));
    app.setOrganizationName(QStringLiteral("Kit3713"));
    app.setOrganizationDomain(QStringLiteral("github.com"));

    // TODO: ship a real icon in resources/proteus.svg and load it here.
    // app.setWindowIcon(QIcon::fromTheme(QStringLiteral("proteus")));

    proteus::MainWindow w;
    w.show();

    return app.exec();
}
