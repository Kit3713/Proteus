// SPDX-License-Identifier: GPL-3.0-or-later
//
// TrayApp.cpp — implementation of the tray icon + state machine.
//
// This is a SKELETON.  See the TODO comments for deferred behaviour.

#include "TrayApp.h"
#include "PopupWindow.h"

#include <QAction>
#include <QApplication>
#include <QIcon>
#include <QJsonArray>
#include <QJsonObject>
#include <QMenu>
#include <QProcess>
#include <QStandardPaths>
#include <QString>
#include <QTimer>

namespace Proteus {

TrayApp::TrayApp(QObject *parent)
    : QObject(parent)
{
    // Wire up the mutate feedback signal so we can re-poll after each action.
    connect(&m_mutator, &ProteusMutate::finished,
            this, &TrayApp::onMutateFinished);

    // Wire up the async status query result.
    connect(&m_runner, &ProteusRunner::queryFinished,
            this, &TrayApp::onStatusResult);
}

TrayApp::~TrayApp()
{
    // m_popup is parented to nullptr (so it appears as a real top-level window)
    // and QPointer does not take ownership, so we must delete it explicitly.
    delete m_popup;
}

void TrayApp::init()
{
    buildMenu();

    // ── Tray icon ─────────────────────────────────────────────────────────
    // Use a named icon from the XDG icon theme; fall back to a stock icon.
    // TODO(roadmap-1.3.x): ship a proper Proteus SVG icon in the package.
    QIcon icon = QIcon::fromTheme(QStringLiteral("network-wired"),
                                   QIcon::fromTheme(QStringLiteral("dialog-information")));
    m_trayIcon.setIcon(icon);
    m_trayIcon.setContextMenu(&m_menu);
    m_trayIcon.setToolTip(QStringLiteral("Proteus — status unknown"));

    connect(&m_trayIcon, &QSystemTrayIcon::activated,
            this, &TrayApp::onTrayActivated);

    m_trayIcon.show();

    // ── Poll timer ────────────────────────────────────────────────────────
    connect(&m_pollTimer, &QTimer::timeout, this, &TrayApp::onPollTimer);
    m_pollTimer.setInterval(kPollIntervalMs);
    m_pollTimer.start();

    // Fire immediately so the icon reflects state at startup.
    QTimer::singleShot(0, this, &TrayApp::onPollTimer);
}

// ── Menu construction ─────────────────────────────────────────────────────────

void TrayApp::buildMenu()
{
    m_actRotate = m_menu.addAction(QStringLiteral("Rotate identifiers"),
                                    this, &TrayApp::onRotateTriggered);

    m_actKill = m_menu.addAction(QStringLiteral("Kill switch"),
                                  this, &TrayApp::onKillTriggered);

    // Look for proteus-gui in PATH; hide the action if not installed.
    m_actOpenProteus = m_menu.addAction(QStringLiteral("Open Proteus…"),
                                         this, &TrayApp::onOpenProteusTriggered);
    m_actOpenProteus->setVisible(!findGuiBinary().isEmpty());

    m_menu.addSeparator();

    m_actQuit = m_menu.addAction(QStringLiteral("Quit tray"),
                                  this, &TrayApp::onQuitTriggered);
}

// ── Poll cycle ────────────────────────────────────────────────────────────────

void TrayApp::onPollTimer()
{
    // Async so we don't block the GUI thread.
    m_runner.queryAsync({QStringLiteral("status"), QStringLiteral("--json")});
}

void TrayApp::onStatusResult(const Proteus::RunResult &result)
{
    if (!result.ok) {
        applyState(ProteusState::Unknown, {});
        return;
    }

    // Derive high-level state from the JSON.
    // `proteus status --json` emits a StatusReport with a top-level
    // "features" array; each entry has a "state" string field.
    //
    // TODO(roadmap-1.3.x): Validate against the JSON Schema once one is
    // published alongside the CLI.  For now, do a best-effort parse.
    const QJsonObject root = result.doc.object();
    const QJsonArray features = root.value(QStringLiteral("features")).toArray();

    int applied = 0, total = 0;
    for (const auto &f : features) {
        const QString state = f.toObject().value(QStringLiteral("state")).toString();
        if (!state.isEmpty()) {
            ++total;
            if (state == QStringLiteral("applied") ||
                state == QStringLiteral("active"))
                ++applied;
        }
    }

    ProteusState derived = ProteusState::Unknown;
    if (total > 0) {
        if (applied == total)
            derived = ProteusState::Applied;
        else if (applied == 0)
            derived = ProteusState::Reverted;
        else
            derived = ProteusState::Partial;
    }

    applyState(derived, result.doc);
}

// ── State application ─────────────────────────────────────────────────────────

void TrayApp::applyState(ProteusState state, const QJsonDocument &doc)
{
    m_state = state;
    updateIcon(state);
    updateTooltip(state, doc);
}

void TrayApp::updateIcon(ProteusState state)
{
    // TODO(roadmap-1.3.x): Use custom per-state SVG icons (green dot /
    // orange dot / grey dot) once artwork is available.  For now, reflect
    // state through the tooltip and a placeholder stock icon.
    Q_UNUSED(state)
    // Icon already set in init(); no per-state change in skeleton.
}

void TrayApp::updateTooltip(ProteusState state, const QJsonDocument &doc)
{
    const QString version = doc.object()
        .value(QStringLiteral("proteus_version")).toString(QStringLiteral("?"));

    QString stateStr;
    switch (state) {
    case ProteusState::Applied:  stateStr = QStringLiteral("applied (all active)");  break;
    case ProteusState::Partial:  stateStr = QStringLiteral("partial");               break;
    case ProteusState::Reverted: stateStr = QStringLiteral("reverted");              break;
    case ProteusState::Unknown:  stateStr = QStringLiteral("unknown / unreachable"); break;
    }

    m_trayIcon.setToolTip(
        QStringLiteral("Proteus %1 — %2").arg(version, stateStr));
}

// ── Menu actions ──────────────────────────────────────────────────────────────

void TrayApp::onRotateTriggered()
{
    // CODE PATH ONLY — never executed in this skeleton build.
    // pkexec proteus rotate --yes
    m_mutator.rotate();
}

void TrayApp::onKillTriggered()
{
    // CODE PATH ONLY — never executed in this skeleton build.
    // pkexec proteus kill --yes
    m_mutator.kill();
}

void TrayApp::onOpenProteusTriggered()
{
    const QString gui = findGuiBinary();
    if (gui.isEmpty()) {
        // Action should already be hidden; this is a safety guard.
        return;
    }
    // TODO(roadmap-1.3.x): Launch as a proper desktop app via QProcess or
    // D-Bus activation rather than a bare fork.
    QProcess::startDetached(gui, {});
}

void TrayApp::onQuitTriggered()
{
    QApplication::quit();
}

// ── Tray activation ───────────────────────────────────────────────────────────

void TrayApp::onTrayActivated(QSystemTrayIcon::ActivationReason reason)
{
    if (reason != QSystemTrayIcon::Trigger)
        return; // Middle-click / double-click / context already handled.

    // Lazily create the popup window on left-click.
    // Parent is nullptr so Qt renders it as a top-level window, but we
    // hold it in a QPointer and explicitly delete it in the destructor
    // to avoid a leak (QPointer does not own the object).
    if (!m_popup) {
        m_popup = new PopupWindow(nullptr);
    }

    if (m_popup->isVisible()) {
        m_popup->hide();
    } else {
        m_popup->refresh(); // pull fresh data before showing
        m_popup->show();
        m_popup->raise();
        m_popup->activateWindow();
    }
}

// ── Mutate feedback ───────────────────────────────────────────────────────────

void TrayApp::onMutateFinished(const QString &subcommand, int exitCode)
{
    // 126 = polkit auth cancelled; 127 = binary not found.
    if (exitCode == 126 || exitCode == 127) {
        // TODO(roadmap-1.3.x): show a non-intrusive notification.
        return;
    }
    Q_UNUSED(subcommand)

    // Re-poll immediately so the icon updates without waiting for the timer.
    onPollTimer();
}

// ── Helpers ───────────────────────────────────────────────────────────────────

QString TrayApp::findGuiBinary()
{
    return QStandardPaths::findExecutable(QStringLiteral("proteus-gui"));
}

} // namespace Proteus
