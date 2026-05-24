// SPDX-License-Identifier: GPL-3.0-or-later
//
// MainWindow.h — top-level application window.
//
// Layout
// ──────
// The window uses a two-panel layout:
//   Left  │ Right
//   ──────┼──────────────────────────
//   nav   │ QStackedWidget (pages)
//   list  │
//
// Navigation items (mirroring the CLI surface — no scope drift):
//   0 — Status dashboard    (proteus status --json)
//   1 — Personas            (proteus persona list/show/use/random/clear --json)
//   2 — Per-SSID rules      (proteus ssid list/show/set/clear --json)
//   3 — Doctor / tools      (proteus doctor/diff/dry-run; apply/revert via pkexec)
//
// When HAVE_KIRIGAMI is defined (Kirigami present at build time) the nav list
// is replaced with a Kirigami.NavigationTabBar or equivalent — the page
// objects are unchanged.

#pragma once

#include <QMainWindow>
#include <QListWidget>
#include <QStackedWidget>
#include <QSplitter>

namespace proteus {

class StatusPage;
class PersonaPage;
class SsidPage;
class DoctorPage;

class MainWindow : public QMainWindow {
    Q_OBJECT
public:
    explicit MainWindow(QWidget *parent = nullptr);
    ~MainWindow() override = default;

private slots:
    void onNavChanged(int row);

private:
    void setupUi();
    void setupNav();

    QSplitter      *m_splitter  = nullptr;
    QListWidget    *m_nav       = nullptr;
    QStackedWidget *m_stack     = nullptr;

    // Pages — allocated once, lazily populated on first show.
    StatusPage  *m_statusPage  = nullptr;
    PersonaPage *m_personaPage = nullptr;
    SsidPage    *m_ssidPage    = nullptr;
    DoctorPage  *m_doctorPage  = nullptr;
};

} // namespace proteus
