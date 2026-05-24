// SPDX-License-Identifier: GPL-3.0-or-later
//
// StatusPage.cpp

#include "StatusPage.h"
#include "../client/ProteusRunner.h"

#include <QVBoxLayout>
#include <QHBoxLayout>
#include <QHeaderView>
#include <QJsonObject>
#include <QJsonArray>
#include <QGroupBox>
#include <QFormLayout>

namespace proteus {

StatusPage::StatusPage(QWidget *parent)
    : QWidget(parent)
{
    setupUi();
}

void StatusPage::setupUi()
{
    auto *vbox = new QVBoxLayout(this);

    // ── Version bar ───────────────────────────────────────────────────────
    m_versionLabel = new QLabel(QStringLiteral("Loading…"), this);
    vbox->addWidget(m_versionLabel);

    // ── System detection ─────────────────────────────────────────────────
    auto *sysGroup = new QGroupBox(QStringLiteral("System"), this);
    auto *sysForm  = new QFormLayout(sysGroup);
    m_sysdLabel     = new QLabel(QStringLiteral("—"), sysGroup);
    m_nmLabel       = new QLabel(QStringLiteral("—"), sysGroup);
    m_bluezLabel    = new QLabel(QStringLiteral("—"), sysGroup);
    m_resolvedLabel = new QLabel(QStringLiteral("—"), sysGroup);
    sysForm->addRow(QStringLiteral("systemd:"),          m_sysdLabel);
    sysForm->addRow(QStringLiteral("NetworkManager:"),   m_nmLabel);
    sysForm->addRow(QStringLiteral("BlueZ:"),            m_bluezLabel);
    sysForm->addRow(QStringLiteral("systemd-resolved:"), m_resolvedLabel);
    vbox->addWidget(sysGroup);

    // ── Interface table ───────────────────────────────────────────────────
    auto *ifaceGroup = new QGroupBox(QStringLiteral("Interfaces"), this);
    auto *ifaceVbox  = new QVBoxLayout(ifaceGroup);
    m_ifaceTable = new QTableWidget(0, 4, ifaceGroup);
    m_ifaceTable->setHorizontalHeaderLabels({
        QStringLiteral("Interface"),
        QStringLiteral("MAC"),
        QStringLiteral("Kind"),
        QStringLiteral("Chipset"),
    });
    m_ifaceTable->horizontalHeader()->setStretchLastSection(true);
    m_ifaceTable->setEditTriggers(QAbstractItemView::NoEditTriggers);
    m_ifaceTable->setSelectionMode(QAbstractItemView::SingleSelection);
    ifaceVbox->addWidget(m_ifaceTable);
    vbox->addWidget(ifaceGroup);

    // ── Feature status table ──────────────────────────────────────────────
    auto *featGroup = new QGroupBox(QStringLiteral("Feature Status"), this);
    auto *featVbox  = new QVBoxLayout(featGroup);
    m_featureTable = new QTableWidget(0, 3, featGroup);
    m_featureTable->setHorizontalHeaderLabels({
        QStringLiteral("Feature"),
        QStringLiteral("State"),
        QStringLiteral("Note"),
    });
    m_featureTable->horizontalHeader()->setStretchLastSection(true);
    m_featureTable->setEditTriggers(QAbstractItemView::NoEditTriggers);
    featVbox->addWidget(m_featureTable);
    vbox->addWidget(featGroup);

    // ── Live event feed placeholder ───────────────────────────────────────
    // TODO (roadmap 1.4.1): replace this static placeholder with a real
    // live feed.  The events daemon will expose a domain-socket or
    // D-Bus signal surface; connect a QLocalSocket / QDBusInterface here
    // and append each event line to m_eventFeed.
    auto *evGroup = new QGroupBox(QStringLiteral("Live event feed (placeholder — roadmap 1.4.1)"), this);
    auto *evVbox  = new QVBoxLayout(evGroup);
    m_eventFeed = new QTextEdit(evGroup);
    m_eventFeed->setReadOnly(true);
    m_eventFeed->setPlaceholderText(
        QStringLiteral("Event feed not yet connected.\n"
                       "Roadmap 1.4.1 will wire this to the proteus events daemon."));
    evVbox->addWidget(m_eventFeed);
    vbox->addWidget(evGroup);

    // ── Error label (hidden until an error occurs) ────────────────────────
    m_errorLabel = new QLabel(this);
    m_errorLabel->setStyleSheet(QStringLiteral("color: red;"));
    m_errorLabel->setVisible(false);
    vbox->addWidget(m_errorLabel);
}

void StatusPage::refresh()
{
    m_errorLabel->setVisible(false);
    m_versionLabel->setText(QStringLiteral("Refreshing…"));

    auto result = ProteusRunner::status();
    if (!result.ok) {
        m_errorLabel->setText(
            QStringLiteral("proteus status failed: %1").arg(result.error));
        m_errorLabel->setVisible(true);
        m_versionLabel->setText(QStringLiteral("(error)"));
        return;
    }
    populate(result.json);
}

void StatusPage::populate(const QJsonDocument &doc)
{
    QJsonObject root = doc.object();

    // Version / phase
    QString version = root.value(QStringLiteral("proteus_version")).toString();
    QString phase   = root.value(QStringLiteral("phase")).toString();
    m_versionLabel->setText(
        QStringLiteral("Proteus %1 (phase: %2)").arg(version, phase));

    // System detection
    QJsonObject sys = root.value(QStringLiteral("system")).toObject();
    auto boolStr = [](bool v) { return v ? QStringLiteral("yes") : QStringLiteral("no"); };
    m_sysdLabel->setText(boolStr(sys.value(QStringLiteral("systemd")).toBool()));
    m_nmLabel->setText(boolStr(sys.value(QStringLiteral("network_manager")).toBool()));
    m_bluezLabel->setText(boolStr(sys.value(QStringLiteral("bluez")).toBool()));
    m_resolvedLabel->setText(boolStr(sys.value(QStringLiteral("systemd_resolved")).toBool()));

    // Interface table
    QJsonArray ifaces = root.value(QStringLiteral("interfaces")).toArray();
    m_ifaceTable->setRowCount(static_cast<int>(ifaces.size()));
    for (int i = 0; i < ifaces.size(); ++i) {
        QJsonObject iface = ifaces[i].toObject();
        m_ifaceTable->setItem(i, 0, new QTableWidgetItem(
            iface.value(QStringLiteral("name")).toString()));
        m_ifaceTable->setItem(i, 1, new QTableWidgetItem(
            iface.value(QStringLiteral("mac")).toString()));
        m_ifaceTable->setItem(i, 2, new QTableWidgetItem(
            iface.value(QStringLiteral("kind")).toString()));

        // chipset sub-object is optional
        QString chipStr;
        QJsonValue chipV = iface.value(QStringLiteral("chipset"));
        if (!chipV.isNull() && chipV.isObject()) {
            QJsonObject chip = chipV.toObject();
            chipStr = QStringLiteral("%1 / %2 / %3")
                .arg(chip.value(QStringLiteral("driver")).toString(QStringLiteral("?")))
                .arg(chip.value(QStringLiteral("chip")).toString(QStringLiteral("?")))
                .arg(chip.value(QStringLiteral("firmware")).toString(QStringLiteral("?")));
        }
        m_ifaceTable->setItem(i, 3, new QTableWidgetItem(chipStr));
    }

    // Feature status table
    QJsonArray features = root.value(QStringLiteral("features")).toArray();
    m_featureTable->setRowCount(static_cast<int>(features.size()));
    for (int i = 0; i < features.size(); ++i) {
        QJsonObject f = features[i].toObject();
        m_featureTable->setItem(i, 0, new QTableWidgetItem(
            f.value(QStringLiteral("name")).toString()));
        m_featureTable->setItem(i, 1, new QTableWidgetItem(
            f.value(QStringLiteral("state")).toString()));
        m_featureTable->setItem(i, 2, new QTableWidgetItem(
            f.value(QStringLiteral("note")).toString()));
    }
}

} // namespace proteus
