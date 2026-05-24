// SPDX-License-Identifier: GPL-3.0-or-later
//
// PopupWindow.cpp — SKELETON implementation.
//
// The window is functional enough to open and close.  Content is a
// placeholder label until roadmap 1.3.x fills in the real widgets.

#include "PopupWindow.h"

#include <QFocusEvent>
#include <QJsonObject>
#include <QLabel>
#include <QVBoxLayout>

PopupWindow::PopupWindow(QWidget *parent)
    : QWidget(parent,
              // Normal window flags: title bar, no layer-shell, works on
              // GNOME / KDE / Xfce / Sway (XWayland) uniformly.
              Qt::Window | Qt::WindowStaysOnTopHint)
{
    setWindowTitle(QStringLiteral("Proteus"));
    setMinimumSize(320, 160);

    connect(&m_runner, &Proteus::ProteusRunner::queryFinished,
            this, &PopupWindow::onStatusResult);

    buildLayout();
}

// ── Layout ────────────────────────────────────────────────────────────────────

void PopupWindow::buildLayout()
{
    auto *layout = new QVBoxLayout(this);

    m_labelVersion = new QLabel(QStringLiteral("Proteus (loading…)"), this);
    m_labelState   = new QLabel(QStringLiteral("State: unknown"), this);
    m_labelPersona = new QLabel(QStringLiteral("Persona: —"), this);

    layout->addWidget(m_labelVersion);
    layout->addWidget(m_labelState);
    layout->addWidget(m_labelPersona);

    // TODO(roadmap-1.3.x): add the full rich layout described in PopupWindow.h
    layout->addStretch();
    setLayout(layout);
}

// ── Refresh ───────────────────────────────────────────────────────────────────

void PopupWindow::refresh()
{
    // Async fetch — onStatusResult() updates the labels when done.
    m_runner.queryAsync({QStringLiteral("status"), QStringLiteral("--json")});
}

void PopupWindow::onStatusResult(const Proteus::RunResult &result)
{
    if (!result.ok) {
        m_labelState->setText(QStringLiteral("State: unreachable (%1)")
            .arg(result.errorString));
        return;
    }
    updateContent(result.doc);
}

void PopupWindow::updateContent(const QJsonDocument &doc)
{
    const QJsonObject root = doc.object();

    const QString ver = root.value(QStringLiteral("proteus_version"))
        .toString(QStringLiteral("?"));
    m_labelVersion->setText(QStringLiteral("Proteus %1").arg(ver));

    // TODO(roadmap-1.3.x): parse features array and render per-feature dots.
    m_labelState->setText(QStringLiteral("State: (full display TODO)"));

    // TODO(roadmap-1.3.x): surface active persona from JSON when the
    //   `proteus status --json` output includes it.
    m_labelPersona->setText(QStringLiteral("Persona: (TODO)"));
}

// ── Focus handling ────────────────────────────────────────────────────────────

void PopupWindow::focusOutEvent(QFocusEvent *event)
{
    // Auto-hide when the user clicks away — matches typical tray popup UX.
    // TODO(roadmap-1.3.x): guard this so the window doesn't disappear when
    //   a polkit dialog (child process) steals focus.
    hide();
    QWidget::focusOutEvent(event);
}
