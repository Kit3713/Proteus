// SPDX-License-Identifier: GPL-3.0-or-later
//
// TrayApp.h — main tray application object.
//
// Responsibilities:
//  1. Registers a QSystemTrayIcon (StatusNotifierItem-compatible on KDE/Xfce;
//     falls back to a hidden icon on GNOME without the extension).
//  2. Polls `proteus status --json` on a timer to track applied/partial/
//     reverted state and updates the icon dot + tooltip accordingly.
//  3. Owns the context menu (Rotate / Kill / Open Proteus).
//  4. Lazily creates and shows the PopupWindow on left-click.
//
// State machine:
//
//   Unknown ──(poll)──> Applied | Partial | Reverted
//            ←──(mutate finished + re-poll)──
//
// Mutate operations are dispatched via ProteusMutate; after each mutate
// the status poll fires immediately to refresh the icon.

#pragma once

#include <QMenu>
#include <QObject>
#include <QPointer>
#include <QSystemTrayIcon>
#include <QTimer>

#include "ProteusRunner.h"
#include "ProteusMutate.h"

// Forward declaration — see PopupWindow.h
class PopupWindow;

namespace Proteus {

/// High-level state derived from `proteus status --json`.
enum class ProteusState {
    Unknown,   ///< Not yet polled, or poll failed.
    Applied,   ///< All configured features are active.
    Partial,   ///< Some features active, some not.
    Reverted,  ///< Proteus is not modifying the system.
};

///
/// TrayApp
///
/// Singleton-ish application object.  Owns the tray icon, menu, popup,
/// runner, and mutator.  Created in main() and lives for the duration of
/// the process.
///
class TrayApp : public QObject {
    Q_OBJECT

public:
    explicit TrayApp(QObject *parent = nullptr);
    ~TrayApp() override;

    // ── Initialisation ──────────────────────────────────────────────────────

    /// Call once after construction to build the UI and start polling.
    void init();

private slots:
    // ── Poll ────────────────────────────────────────────────────────────────

    /// Triggered by m_pollTimer; refreshes state from the CLI.
    void onPollTimer();

    /// Handles the completed status query.
    void onStatusResult(const Proteus::RunResult &result);

    // ── Menu actions ────────────────────────────────────────────────────────

    void onRotateTriggered();
    void onKillTriggered();
    void onOpenProteusTriggered();   ///< PATH-probes for proteus-gui; hidden if absent.
    void onQuitTriggered();

    // ── Tray interactions ───────────────────────────────────────────────────

    void onTrayActivated(QSystemTrayIcon::ActivationReason reason);

    // ── Mutate feedback ─────────────────────────────────────────────────────

    void onMutateFinished(const QString &subcommand, int exitCode);

private:
    void buildMenu();
    void applyState(ProteusState state, const QJsonDocument &doc);
    void updateIcon(ProteusState state);
    void updateTooltip(ProteusState state, const QJsonDocument &doc);

    /// Look for proteus-gui in PATH.  Returns the full path, or empty string.
    static QString findGuiBinary();

    // ── Owned children ──────────────────────────────────────────────────────

    QSystemTrayIcon m_trayIcon;
    QMenu           m_menu;
    QTimer          m_pollTimer;

    QPointer<PopupWindow> m_popup; ///< lazily created on first left-click

    ProteusRunner  m_runner;
    ProteusMutate  m_mutator;

    // ── Menu actions (raw ptrs — owned by m_menu) ───────────────────────────
    QAction *m_actRotate      = nullptr;
    QAction *m_actKill        = nullptr;
    QAction *m_actOpenProteus = nullptr;  ///< hidden when proteus-gui not found
    QAction *m_actQuit        = nullptr;

    ProteusState m_state = ProteusState::Unknown;

    // Poll cadence (ms).  TODO(roadmap-1.3.x): make configurable.
    static constexpr int kPollIntervalMs = 10'000;
};

} // namespace Proteus
