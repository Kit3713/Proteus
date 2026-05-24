// SPDX-License-Identifier: GPL-3.0-or-later
//
// PersonaPage.h — Persona gallery.
//
// Reads:
//   proteus persona list   --json
//   proteus persona show <id> --json
//
// Mutations (via pkexec):
//   proteus persona use    <id> --yes
//   proteus persona random      --yes
//   proteus persona clear       --yes
//
// Layout: left list of personas; right detail panel; bottom action bar.
//
// TODO (1.4.1): add persona new / edit / import / export surfaces.

#pragma once

#include <QWidget>
#include <QListWidget>
#include <QLabel>
#include <QPushButton>
#include <QTextBrowser>
#include <QSplitter>

namespace proteus {

class PersonaPage : public QWidget {
    Q_OBJECT
public:
    explicit PersonaPage(QWidget *parent = nullptr);

public slots:
    void refresh();

private slots:
    void onPersonaSelected(QListWidgetItem *item);
    void onUseClicked();
    void onRandomClicked();
    void onClearClicked();

private:
    void setupUi();
    void showDetail(const QString &personaId);
    /// Enable/disable all three mutation buttons.
    /// `useEnabled` is independent: the Use button requires a persona selected.
    void setMutateEnabled(bool enabled, bool useEnabled = false);

    QSplitter    *m_splitter      = nullptr;
    QListWidget  *m_list          = nullptr;
    QTextBrowser *m_detail        = nullptr;
    QPushButton  *m_useBtn        = nullptr;
    QPushButton  *m_randomBtn     = nullptr;
    QPushButton  *m_clearBtn      = nullptr;
    QLabel       *m_errorLabel    = nullptr;
    QLabel       *m_activeLabel   = nullptr;

    QString m_selectedId;  ///< persona id currently selected in m_list
};

} // namespace proteus
