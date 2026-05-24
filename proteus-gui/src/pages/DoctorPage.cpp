// SPDX-License-Identifier: GPL-3.0-or-later

#include "DoctorPage.h"
#include "../client/ProteusRunner.h"
#include "../client/PkexecRunner.h"

#include <QVBoxLayout>
#include <QHBoxLayout>
#include <QHeaderView>
#include <QMessageBox>
#include <QJsonObject>
#include <QJsonArray>

namespace proteus {

DoctorPage::DoctorPage(QWidget *parent)
    : QWidget(parent)
{
    setupUi();
}

void DoctorPage::setupUi()
{
    auto *vbox = new QVBoxLayout(this);

    // ── Tab widget ────────────────────────────────────────────────────────
    m_tabs = new QTabWidget(this);

    // Doctor tab
    auto *doctorWidget = new QWidget(m_tabs);
    auto *doctorVbox   = new QVBoxLayout(doctorWidget);
    m_checkTable = new QTableWidget(0, 4, doctorWidget);
    m_checkTable->setHorizontalHeaderLabels({
        QStringLiteral("Category"),
        QStringLiteral("Check"),
        QStringLiteral("Status"),
        QStringLiteral("Message"),
    });
    m_checkTable->horizontalHeader()->setStretchLastSection(true);
    m_checkTable->setEditTriggers(QAbstractItemView::NoEditTriggers);
    doctorVbox->addWidget(m_checkTable);
    m_tabs->addTab(doctorWidget, QStringLiteral("Doctor"));

    // Diff tab
    auto *diffWidget = new QWidget(m_tabs);
    auto *diffVbox   = new QVBoxLayout(diffWidget);
    m_diffView = new QTextBrowser(diffWidget);
    m_diffView->setPlaceholderText(
        QStringLiteral("Click 'Refresh' to load the current config diff."));
    diffVbox->addWidget(m_diffView);
    m_tabs->addTab(diffWidget, QStringLiteral("Diff"));

    // Dry-run tab
    auto *dryRunWidget = new QWidget(m_tabs);
    auto *dryRunVbox   = new QVBoxLayout(dryRunWidget);
    m_dryRunView = new QTextBrowser(dryRunWidget);
    m_dryRunView->setPlaceholderText(
        QStringLiteral("Click 'Refresh' to see what 'apply' would do without applying it."));
    dryRunVbox->addWidget(m_dryRunView);
    m_tabs->addTab(dryRunWidget, QStringLiteral("Dry-run"));

    vbox->addWidget(m_tabs, 1);

    // ── Mutations bar ─────────────────────────────────────────────────────
    auto *btnRow  = new QHBoxLayout();
    m_applyBtn  = new QPushButton(QStringLiteral("Apply"), this);
    m_revertBtn = new QPushButton(QStringLiteral("Revert"), this);

    // Visually distinguish destructive actions.
    m_revertBtn->setStyleSheet(
        QStringLiteral("QPushButton { color: white; background-color: #c0392b; }"));

    btnRow->addWidget(m_applyBtn);
    btnRow->addWidget(m_revertBtn);
    btnRow->addStretch();
    vbox->addLayout(btnRow);

    // Error label
    m_errorLabel = new QLabel(this);
    m_errorLabel->setStyleSheet(QStringLiteral("color: red;"));
    m_errorLabel->setVisible(false);
    vbox->addWidget(m_errorLabel);

    // Tab changes load data lazily
    connect(m_tabs, &QTabWidget::currentChanged, this, [this](int idx) {
        switch (idx) {
        case 0: onDoctorTabShown();  break;
        case 1: onDiffTabShown();    break;
        case 2: onDryRunTabShown();  break;
        }
    });

    connect(m_applyBtn,  &QPushButton::clicked, this, &DoctorPage::onApplyClicked);
    connect(m_revertBtn, &QPushButton::clicked, this, &DoctorPage::onRevertClicked);
}

void DoctorPage::refresh()
{
    m_errorLabel->setVisible(false);
    // Re-load whichever tab is currently visible.
    switch (m_tabs->currentIndex()) {
    case 0: onDoctorTabShown();  break;
    case 1: onDiffTabShown();    break;
    case 2: onDryRunTabShown();  break;
    }
}

void DoctorPage::onDoctorTabShown()
{
    m_checkTable->setRowCount(0);
    auto result = ProteusRunner::doctor();
    if (!result.ok) {
        m_errorLabel->setText(
            QStringLiteral("doctor failed: %1").arg(result.error));
        m_errorLabel->setVisible(true);
        return;
    }
    m_errorLabel->setVisible(false);

    QJsonObject root   = result.json.object();
    QJsonArray  checks = root.value(QStringLiteral("checks")).toArray();

    m_checkTable->setRowCount(static_cast<int>(checks.size()));
    for (int i = 0; i < checks.size(); ++i) {
        QJsonObject c = checks[i].toObject();
        m_checkTable->setItem(i, 0, new QTableWidgetItem(
            c.value(QStringLiteral("category")).toString()));
        m_checkTable->setItem(i, 1, new QTableWidgetItem(
            c.value(QStringLiteral("name")).toString()));

        QString status = c.value(QStringLiteral("status")).toString();
        auto *statusItem = new QTableWidgetItem(status);
        // Colour-code by status
        if (status == QLatin1String("ok")) {
            statusItem->setForeground(QColor(Qt::darkGreen));
        } else if (status == QLatin1String("warn")) {
            statusItem->setForeground(QColor(Qt::darkYellow));
        } else if (status == QLatin1String("fail")) {
            statusItem->setForeground(QColor(Qt::red));
        }
        m_checkTable->setItem(i, 2, statusItem);
        m_checkTable->setItem(i, 3, new QTableWidgetItem(
            c.value(QStringLiteral("message")).toString()));
    }
}

void DoctorPage::onDiffTabShown()
{
    m_diffView->setPlainText(QStringLiteral("Loading…"));
    auto result = ProteusRunner::diff();
    if (!result.ok) {
        m_diffView->setPlainText(
            QStringLiteral("diff failed: %1").arg(result.error));
        return;
    }
    m_diffView->setPlainText(
        QString::fromUtf8(result.json.toJson(QJsonDocument::Indented)));
}

void DoctorPage::onDryRunTabShown()
{
    m_dryRunView->setPlainText(QStringLiteral("Loading…"));
    // Preview what `proteus apply` would do — the most useful default.
    // dry-run requires an inner subcommand; passing "apply" is equivalent to
    // `proteus dry-run apply --json`.
    auto result = ProteusRunner::dryRun(QStringLiteral("apply"));
    if (!result.ok) {
        m_dryRunView->setPlainText(
            QStringLiteral("dry-run apply failed: %1").arg(result.error));
        return;
    }
    m_dryRunView->setPlainText(
        QString::fromUtf8(result.json.toJson(QJsonDocument::Indented)));
}

// ── Mutations ─────────────────────────────────────────────────────────────────

void DoctorPage::onApplyClicked()
{
    auto ans = QMessageBox::question(
        this,
        QStringLiteral("Apply"),
        QStringLiteral("Apply Proteus config to the running system?\n\n"
                       "This will run:\n  pkexec proteus apply --yes"),
        QMessageBox::Yes | QMessageBox::No);
    if (ans != QMessageBox::Yes) return;

    // TODO (threading): run in a QThread with a progress indicator
    auto result = PkexecRunner::apply();
    if (!result.ok) {
        QMessageBox::critical(this, QStringLiteral("Error"),
            QStringLiteral("apply failed (exit %1):\n%2")
                .arg(result.exitCode).arg(result.output));
        return;
    }
    QMessageBox::information(this, QStringLiteral("Done"),
        QStringLiteral("apply completed successfully."));
    refresh();
}

void DoctorPage::onRevertClicked()
{
    auto ans = QMessageBox::question(
        this,
        QStringLiteral("Revert"),
        QStringLiteral("Revert all Proteus changes?\n\n"
                       "This will run:\n  pkexec proteus revert --yes\n\n"
                       "This undoes apply and restores original identifiers."),
        QMessageBox::Yes | QMessageBox::No);
    if (ans != QMessageBox::Yes) return;

    auto result = PkexecRunner::revert();
    if (!result.ok) {
        QMessageBox::critical(this, QStringLiteral("Error"),
            QStringLiteral("revert failed (exit %1):\n%2")
                .arg(result.exitCode).arg(result.output));
        return;
    }
    QMessageBox::information(this, QStringLiteral("Done"),
        QStringLiteral("revert completed successfully."));
    refresh();
}

} // namespace proteus
