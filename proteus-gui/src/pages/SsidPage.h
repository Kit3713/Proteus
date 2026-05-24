// SPDX-License-Identifier: GPL-3.0-or-later
//
// SsidPage.h — Per-SSID rules table.
//
// Reads:
//   proteus ssid list        --json
//   proteus ssid show <ssid> --json
//
// Mutations (via pkexec):
//   proteus ssid set   <ssid> <key> <value> --yes
//   proteus ssid clear <ssid>               --yes
//
// Layout: top = rules table; right panel = detail on row selection;
//         bottom = set/clear action bar.
//
// SECURITY NOTE: SSIDs are attacker-controlled (hostile APs can broadcast
// arbitrary bytes including ANSI escapes and BiDi overrides).  All SSID
// strings are displayed via QLabel / QTableWidgetItem (Qt escapes HTML) and
// are NEVER passed to QString::arg() in a context that could expand shell
// metacharacters.  When SSIDs flow into PkexecRunner::ssidSet/ssidClear they
// are passed as QStringList elements to QProcess::setArguments, which does
// NOT shell-interpolate them.

#pragma once

#include <QWidget>
#include <QTableWidget>
#include <QTextBrowser>
#include <QPushButton>
#include <QLabel>
#include <QLineEdit>
#include <QSplitter>

namespace proteus {

class SsidPage : public QWidget {
    Q_OBJECT
public:
    explicit SsidPage(QWidget *parent = nullptr);

public slots:
    void refresh();

private slots:
    void onRowSelected(int row, int col);
    void onSetClicked();
    void onClearClicked();

private:
    void setupUi();
    void showDetail(const QString &ssid);

    QSplitter    *m_splitter    = nullptr;
    QTableWidget *m_table       = nullptr;
    QTextBrowser *m_detail      = nullptr;

    // Set-field form
    QLineEdit    *m_keyEdit     = nullptr;
    QLineEdit    *m_valueEdit   = nullptr;
    QPushButton  *m_setBtn      = nullptr;
    QPushButton  *m_clearBtn    = nullptr;
    QLabel       *m_errorLabel  = nullptr;

    QString m_selectedSsid;  ///< raw SSID from JSON (not display-escaped)
};

} // namespace proteus
