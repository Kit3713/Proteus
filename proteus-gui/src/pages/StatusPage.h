// SPDX-License-Identifier: GPL-3.0-or-later
//
// StatusPage.h — Status dashboard.
//
// Reads: `proteus status --json`
// Displays:
//   - proteus version + phase
//   - System detection (systemd / NM / BlueZ / resolved)
//   - Interface table (name, MAC, kind, chipset)
//   - Feature status table (each [section] in config)
//   - Placeholder: live event feed (roadmap 1.4.1 — requires the events
//     daemon websocket/IPC surface, not yet designed).
//
// No mutations on this page.

#pragma once

#include <QWidget>
#include <QLabel>
#include <QTableWidget>
#include <QGroupBox>
#include <QTextEdit>

namespace proteus {

class StatusPage : public QWidget {
    Q_OBJECT
public:
    explicit StatusPage(QWidget *parent = nullptr);

public slots:
    /// Re-run `proteus status --json` and repopulate the UI.
    void refresh();

private:
    void setupUi();
    void populate(const QJsonDocument &doc);

    // Version / phase
    QLabel *m_versionLabel = nullptr;

    // System detection
    QLabel *m_sysdLabel    = nullptr;
    QLabel *m_nmLabel      = nullptr;
    QLabel *m_bluezLabel   = nullptr;
    QLabel *m_resolvedLabel = nullptr;

    // Interface table: columns = name | mac | kind | chipset
    QTableWidget *m_ifaceTable = nullptr;

    // Feature status table: columns = feature | state | note
    QTableWidget *m_featureTable = nullptr;

    // Live event feed placeholder
    QTextEdit *m_eventFeed = nullptr;

    QLabel *m_errorLabel = nullptr;
};

} // namespace proteus
