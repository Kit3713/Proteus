// SPDX-License-Identifier: GPL-3.0-or-later

#include "PersonaPage.h"
#include "../client/ProteusRunner.h"
#include "../client/PkexecRunner.h"

#include <QVBoxLayout>
#include <QHBoxLayout>
#include <QMessageBox>
#include <QJsonObject>
#include <QJsonArray>
#include <QListWidgetItem>

namespace proteus {

PersonaPage::PersonaPage(QWidget *parent)
    : QWidget(parent)
{
    setupUi();
}

void PersonaPage::setupUi()
{
    auto *vbox = new QVBoxLayout(this);

    // Active persona label
    m_activeLabel = new QLabel(QStringLiteral("Active persona: —"), this);
    vbox->addWidget(m_activeLabel);

    // Splitter: list | detail
    m_splitter = new QSplitter(Qt::Horizontal, this);

    m_list = new QListWidget(m_splitter);
    m_list->setMinimumWidth(200);

    m_detail = new QTextBrowser(m_splitter);
    m_detail->setPlaceholderText(
        QStringLiteral("Select a persona to see its details."));

    m_splitter->setStretchFactor(0, 0);
    m_splitter->setStretchFactor(1, 1);
    vbox->addWidget(m_splitter, 1);

    // Action bar
    auto *btnRow = new QHBoxLayout();
    m_useBtn    = new QPushButton(QStringLiteral("Use selected"),  this);
    m_randomBtn = new QPushButton(QStringLiteral("Random"),        this);
    m_clearBtn  = new QPushButton(QStringLiteral("Clear persona"), this);
    setMutateEnabled(false, false);

    btnRow->addWidget(m_useBtn);
    btnRow->addWidget(m_randomBtn);
    btnRow->addWidget(m_clearBtn);
    btnRow->addStretch();
    vbox->addLayout(btnRow);

    // Error label
    m_errorLabel = new QLabel(this);
    m_errorLabel->setStyleSheet(QStringLiteral("color: red;"));
    m_errorLabel->setVisible(false);
    vbox->addWidget(m_errorLabel);

    connect(m_list, &QListWidget::itemClicked,
            this,   &PersonaPage::onPersonaSelected);
    connect(m_useBtn,    &QPushButton::clicked, this, &PersonaPage::onUseClicked);
    connect(m_randomBtn, &QPushButton::clicked, this, &PersonaPage::onRandomClicked);
    connect(m_clearBtn,  &QPushButton::clicked, this, &PersonaPage::onClearClicked);
}

void PersonaPage::refresh()
{
    m_errorLabel->setVisible(false);
    m_list->clear();
    m_detail->clear();
    m_selectedId.clear();
    setMutateEnabled(false, false);

    auto result = ProteusRunner::personaList();
    if (!result.ok) {
        m_errorLabel->setText(
            QStringLiteral("persona list failed: %1").arg(result.error));
        m_errorLabel->setVisible(true);
        return;
    }

    QJsonArray arr = result.json.array();
    for (const QJsonValue &v : arr) {
        QJsonObject p = v.toObject();
        QString id          = p.value(QStringLiteral("id")).toString();
        QString displayName = p.value(QStringLiteral("display_name")).toString(id);
        bool    active      = p.value(QStringLiteral("active")).toBool(false);

        auto *item = new QListWidgetItem(m_list);
        // Display the display_name; store the id as user data for mutations.
        item->setText(active
            ? QStringLiteral("%1 (active)").arg(displayName)
            : displayName);
        item->setData(Qt::UserRole, id);

        if (active) {
            m_activeLabel->setText(
                QStringLiteral("Active persona: %1").arg(displayName));
        }
    }

    // Random and Clear are always available after a successful list.
    // Use requires a selection — leave m_useBtn disabled until the user picks one.
    setMutateEnabled(true, /*useEnabled=*/false);
}

void PersonaPage::onPersonaSelected(QListWidgetItem *item)
{
    if (!item) return;
    m_selectedId = item->data(Qt::UserRole).toString();
    // Now that a persona is selected, enable the Use button.
    m_useBtn->setEnabled(true);
    showDetail(m_selectedId);
}

void PersonaPage::showDetail(const QString &personaId)
{
    m_detail->setPlainText(QStringLiteral("Loading…"));
    auto result = ProteusRunner::personaShow(personaId);
    if (!result.ok) {
        m_detail->setPlainText(
            QStringLiteral("persona show failed: %1").arg(result.error));
        return;
    }

    // Render the JSON as pretty-printed text.
    // TODO (1.4.1): render a structured form (name, description, OUI pool, etc.)
    m_detail->setPlainText(
        QString::fromUtf8(result.json.toJson(QJsonDocument::Indented)));
}

void PersonaPage::onUseClicked()
{
    if (m_selectedId.isEmpty()) return;

    auto ans = QMessageBox::question(
        this,
        QStringLiteral("Use persona"),
        QStringLiteral("Apply persona '%1'?\n\nThis will run:\n  pkexec proteus persona use %1 --yes")
            .arg(m_selectedId),
        QMessageBox::Yes | QMessageBox::No);
    if (ans != QMessageBox::Yes) return;

    // TODO (threading): run PkexecRunner in a QThread / QFuture so the UI
    // doesn't block while pkexec shows its authentication dialog.
    auto result = PkexecRunner::personaUse(m_selectedId);
    if (!result.ok) {
        QMessageBox::critical(this, QStringLiteral("Error"),
            QStringLiteral("persona use failed (exit %1):\n%2")
                .arg(result.exitCode).arg(result.output));
        return;
    }
    refresh();
}

void PersonaPage::onRandomClicked()
{
    auto ans = QMessageBox::question(
        this,
        QStringLiteral("Random persona"),
        QStringLiteral("Apply a random persona?\n\nThis will run:\n  pkexec proteus persona random --yes"),
        QMessageBox::Yes | QMessageBox::No);
    if (ans != QMessageBox::Yes) return;

    auto result = PkexecRunner::personaRandom();
    if (!result.ok) {
        QMessageBox::critical(this, QStringLiteral("Error"),
            QStringLiteral("persona random failed (exit %1):\n%2")
                .arg(result.exitCode).arg(result.output));
        return;
    }
    refresh();
}

void PersonaPage::onClearClicked()
{
    auto ans = QMessageBox::question(
        this,
        QStringLiteral("Clear persona"),
        QStringLiteral("Clear the active persona?\n\nThis will run:\n  pkexec proteus persona clear --yes"),
        QMessageBox::Yes | QMessageBox::No);
    if (ans != QMessageBox::Yes) return;

    auto result = PkexecRunner::personaClear();
    if (!result.ok) {
        QMessageBox::critical(this, QStringLiteral("Error"),
            QStringLiteral("persona clear failed (exit %1):\n%2")
                .arg(result.exitCode).arg(result.output));
        return;
    }
    refresh();
}

void PersonaPage::setMutateEnabled(bool enabled, bool useEnabled)
{
    // Use requires a persona to be selected; Random and Clear do not.
    m_useBtn->setEnabled(enabled && useEnabled);
    m_randomBtn->setEnabled(enabled);
    m_clearBtn->setEnabled(enabled);
}

} // namespace proteus
