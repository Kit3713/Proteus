// SPDX-License-Identifier: GPL-3.0-or-later
//
// DoctorPage.h — Doctor / tools surface.
//
// Reads (no elevation):
//   proteus doctor   --json   → check results table
//   proteus diff     --json   → config diff viewer
//   proteus dry-run  --json   → what apply would do (without doing it)
//
// Mutations (via pkexec):
//   proteus apply  --yes
//   proteus revert --yes
//
// Layout: tab widget with Doctor | Diff | Dry-run tabs; bottom = Apply/Revert.

#pragma once

#include <QWidget>
#include <QTabWidget>
#include <QTableWidget>
#include <QTextBrowser>
#include <QPushButton>
#include <QLabel>

namespace proteus {

class DoctorPage : public QWidget {
    Q_OBJECT
public:
    explicit DoctorPage(QWidget *parent = nullptr);

public slots:
    void refresh();

private slots:
    void onDoctorTabShown();
    void onDiffTabShown();
    void onDryRunTabShown();
    void onApplyClicked();
    void onRevertClicked();

private:
    void setupUi();

    QTabWidget   *m_tabs         = nullptr;

    // Doctor tab
    QTableWidget *m_checkTable   = nullptr;

    // Diff tab
    QTextBrowser *m_diffView     = nullptr;

    // Dry-run tab
    QTextBrowser *m_dryRunView   = nullptr;

    // Mutations bar
    QPushButton  *m_applyBtn     = nullptr;
    QPushButton  *m_revertBtn    = nullptr;

    QLabel       *m_errorLabel   = nullptr;
};

} // namespace proteus
