// SPDX-License-Identifier: GPL-3.0-or-later
//
// PopupWindow.h — lazily-created "rich popup" window stub.
//
// This is a cross-DE normal top-level window (NOT a layer-shell surface).
// It is shown/hidden on tray left-click.  No compositor protocol is required.
//
// Intended content (roadmap 1.3.x):
//  • Current network name (SSID) + connection type
//  • Active persona name (or "none")
//  • Per-feature status dots (applied / partial / reverted)
//  • Quick action buttons (Rotate / Kill / Apply) — wired via ProteusMutate
//
// In this skeleton the window shows a placeholder label only.

#pragma once

#include <QLabel>
#include <QWidget>

#include "ProteusRunner.h"

///
/// PopupWindow
///
/// Frameless (or lightly framed) top-level widget.  Shown on tray left-click,
/// hidden when clicked again or when it loses focus.
///
class PopupWindow : public QWidget {
    Q_OBJECT

public:
    explicit PopupWindow(QWidget *parent = nullptr);

    /// Re-query the CLI and update displayed content.
    void refresh();

protected:
    void focusOutEvent(QFocusEvent *event) override;

private slots:
    void onStatusResult(const Proteus::RunResult &result);

private:
    void buildLayout();
    void updateContent(const QJsonDocument &doc);

    Proteus::ProteusRunner m_runner;

    // ── Widgets (owned by this) ─────────────────────────────────────────────
    QLabel *m_labelVersion  = nullptr;
    QLabel *m_labelState    = nullptr;
    QLabel *m_labelPersona  = nullptr;

    // TODO(roadmap-1.3.x): Replace placeholder labels with a proper layout:
    //   QVBoxLayout with:
    //     - Network status row (icon + SSID + connection type)
    //     - Persona row (active persona name + OUI pool)
    //     - Feature grid (per-feature dot + name + applied/reverted)
    //     - Quick action bar (Rotate / Kill / Apply buttons)
};
