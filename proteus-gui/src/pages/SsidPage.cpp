// SPDX-License-Identifier: GPL-3.0-or-later

#include "SsidPage.h"
#include "../client/ProteusRunner.h"
#include "../client/PkexecRunner.h"

#include <QVBoxLayout>
#include <QHBoxLayout>
#include <QFormLayout>
#include <QGroupBox>
#include <QHeaderView>
#include <QMessageBox>
#include <QJsonObject>
#include <QJsonArray>

namespace proteus {

SsidPage::SsidPage(QWidget *parent)
    : QWidget(parent)
{
    setupUi();
}

void SsidPage::setupUi()
{
    auto *vbox = new QVBoxLayout(this);

    // Splitter: table | detail
    m_splitter = new QSplitter(Qt::Horizontal, this);

    // Left: rules table
    m_table = new QTableWidget(0, 6, m_splitter);
    m_table->setHorizontalHeaderLabels({
        QStringLiteral("SSID"),
        QStringLiteral("Persona"),
        QStringLiteral("Profile"),
        QStringLiteral("Pin MAC"),
        QStringLiteral("Rotate"),
        QStringLiteral("Portal policy"),
    });
    m_table->horizontalHeader()->setStretchLastSection(true);
    m_table->setEditTriggers(QAbstractItemView::NoEditTriggers);
    m_table->setSelectionBehavior(QAbstractItemView::SelectRows);
    m_table->setSelectionMode(QAbstractItemView::SingleSelection);

    // Right: detail
    m_detail = new QTextBrowser(m_splitter);
    m_detail->setPlaceholderText(
        QStringLiteral("Select a row to see full SSID rule detail."));

    m_splitter->setStretchFactor(0, 1);
    m_splitter->setStretchFactor(1, 0);
    vbox->addWidget(m_splitter, 1);

    // ── Set/clear action bar ──────────────────────────────────────────────
    auto *actGroup = new QGroupBox(QStringLiteral("Edit selected SSID rule"), this);
    auto *actLayout = new QFormLayout(actGroup);

    m_keyEdit   = new QLineEdit(actGroup);
    m_keyEdit->setPlaceholderText(
        QStringLiteral("e.g. persona, aggressiveness_profile, pin_mac, rotate_interval, portal_policy"));
    m_valueEdit = new QLineEdit(actGroup);
    m_valueEdit->setPlaceholderText(QStringLiteral("new value"));

    actLayout->addRow(QStringLiteral("Key:"),   m_keyEdit);
    actLayout->addRow(QStringLiteral("Value:"), m_valueEdit);

    auto *btnRow = new QHBoxLayout();
    m_setBtn   = new QPushButton(QStringLiteral("Set"),   this);
    m_clearBtn = new QPushButton(QStringLiteral("Clear all rules for SSID"), this);
    m_setBtn->setEnabled(false);
    m_clearBtn->setEnabled(false);
    btnRow->addWidget(m_setBtn);
    btnRow->addWidget(m_clearBtn);
    btnRow->addStretch();
    actLayout->addRow(btnRow);

    vbox->addWidget(actGroup);

    // Error label
    m_errorLabel = new QLabel(this);
    m_errorLabel->setStyleSheet(QStringLiteral("color: red;"));
    m_errorLabel->setVisible(false);
    vbox->addWidget(m_errorLabel);

    connect(m_table, &QTableWidget::cellClicked,
            this,    &SsidPage::onRowSelected);
    connect(m_setBtn,   &QPushButton::clicked, this, &SsidPage::onSetClicked);
    connect(m_clearBtn, &QPushButton::clicked, this, &SsidPage::onClearClicked);
}

void SsidPage::refresh()
{
    m_errorLabel->setVisible(false);
    m_table->setRowCount(0);
    m_detail->clear();
    m_selectedSsid.clear();
    m_setBtn->setEnabled(false);
    m_clearBtn->setEnabled(false);

    auto result = ProteusRunner::ssidList();
    if (!result.ok) {
        m_errorLabel->setText(
            QStringLiteral("ssid list failed: %1").arg(result.error));
        m_errorLabel->setVisible(true);
        return;
    }

    QJsonArray arr = result.json.array();
    m_table->setRowCount(static_cast<int>(arr.size()));

    for (int i = 0; i < arr.size(); ++i) {
        QJsonObject e = arr[i].toObject();
        // Store raw SSID as column 0 user data; display text is the same
        // (Qt's QTableWidgetItem escapes HTML, so no injection risk).
        QString ssid = e.value(QStringLiteral("ssid")).toString();

        auto *ssidItem = new QTableWidgetItem(ssid);
        ssidItem->setData(Qt::UserRole, ssid);  // raw for mutations
        m_table->setItem(i, 0, ssidItem);
        m_table->setItem(i, 1, new QTableWidgetItem(
            e.value(QStringLiteral("persona")).toString()));
        m_table->setItem(i, 2, new QTableWidgetItem(
            e.value(QStringLiteral("aggressiveness_profile")).toString()));
        m_table->setItem(i, 3, new QTableWidgetItem(
            e.value(QStringLiteral("pin_mac")).toString()));
        m_table->setItem(i, 4, new QTableWidgetItem(
            e.value(QStringLiteral("rotate_interval")).toString()));
        m_table->setItem(i, 5, new QTableWidgetItem(
            e.value(QStringLiteral("portal_policy")).toString()));
    }
}

void SsidPage::onRowSelected(int row, int /*col*/)
{
    auto *item = m_table->item(row, 0);
    if (!item) return;
    m_selectedSsid = item->data(Qt::UserRole).toString();
    m_setBtn->setEnabled(true);
    m_clearBtn->setEnabled(true);
    showDetail(m_selectedSsid);
}

void SsidPage::showDetail(const QString &ssid)
{
    m_detail->setPlainText(QStringLiteral("Loading…"));
    auto result = ProteusRunner::ssidShow(ssid);
    if (!result.ok) {
        m_detail->setPlainText(
            QStringLiteral("ssid show failed: %1").arg(result.error));
        return;
    }
    m_detail->setPlainText(
        QString::fromUtf8(result.json.toJson(QJsonDocument::Indented)));
}

void SsidPage::onSetClicked()
{
    if (m_selectedSsid.isEmpty()) return;

    QString key   = m_keyEdit->text().trimmed();
    QString value = m_valueEdit->text().trimmed();
    if (key.isEmpty() || value.isEmpty()) {
        QMessageBox::warning(this, QStringLiteral("Input required"),
            QStringLiteral("Both key and value must be filled in."));
        return;
    }

    auto ans = QMessageBox::question(
        this,
        QStringLiteral("Set SSID rule"),
        QStringLiteral("Set rule for SSID?\n\n"
                       "  pkexec proteus ssid set \"<ssid>\" %1 %2 --yes")
            .arg(key, value),
        QMessageBox::Yes | QMessageBox::No);
    if (ans != QMessageBox::Yes) return;

    // TODO (threading): run in a QThread
    auto result = PkexecRunner::ssidSet(m_selectedSsid, key, value);
    if (!result.ok) {
        QMessageBox::critical(this, QStringLiteral("Error"),
            QStringLiteral("ssid set failed (exit %1):\n%2")
                .arg(result.exitCode).arg(result.output));
        return;
    }
    refresh();
}

void SsidPage::onClearClicked()
{
    if (m_selectedSsid.isEmpty()) return;

    auto ans = QMessageBox::question(
        this,
        QStringLiteral("Clear SSID rules"),
        QStringLiteral("Clear ALL rules for this SSID?\n\n"
                       "  pkexec proteus ssid clear \"<ssid>\" --yes"),
        QMessageBox::Yes | QMessageBox::No);
    if (ans != QMessageBox::Yes) return;

    auto result = PkexecRunner::ssidClear(m_selectedSsid);
    if (!result.ok) {
        QMessageBox::critical(this, QStringLiteral("Error"),
            QStringLiteral("ssid clear failed (exit %1):\n%2")
                .arg(result.exitCode).arg(result.output));
        return;
    }
    refresh();
}

} // namespace proteus
