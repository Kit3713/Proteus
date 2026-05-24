// SPDX-License-Identifier: GPL-3.0-or-later

#include "MainWindow.h"

#include "pages/StatusPage.h"
#include "pages/PersonaPage.h"
#include "pages/SsidPage.h"
#include "pages/DoctorPage.h"

#include <QHBoxLayout>
#include <QLabel>
#include <QStatusBar>

namespace proteus {

MainWindow::MainWindow(QWidget *parent)
    : QMainWindow(parent)
{
    setWindowTitle(QStringLiteral("Proteus"));
    resize(1000, 680);
    setupUi();
    setupNav();
    // Show the Status dashboard by default.
    m_nav->setCurrentRow(0);
}

void MainWindow::setupUi()
{
    // Central widget holds the splitter.
    auto *central = new QWidget(this);
    setCentralWidget(central);

    auto *hbox = new QHBoxLayout(central);
    hbox->setContentsMargins(0, 0, 0, 0);
    hbox->setSpacing(0);

    m_splitter = new QSplitter(Qt::Horizontal, central);
    hbox->addWidget(m_splitter);

    // ── Left: navigation list ─────────────────────────────────────────────
    m_nav = new QListWidget(m_splitter);
    m_nav->setFixedWidth(180);
    m_nav->setFrameShape(QFrame::NoFrame);
    // TODO (Kirigami): when HAVE_KIRIGAMI, replace m_nav with a
    //   Kirigami.NavigationTabBar declared in a QML component and promoted
    //   via QQuickWidget into this slot.

    // ── Right: page stack ─────────────────────────────────────────────────
    m_stack = new QStackedWidget(m_splitter);

    m_splitter->setStretchFactor(0, 0);  // nav: fixed
    m_splitter->setStretchFactor(1, 1);  // stack: expands

    // ── Pages (allocated here; data loaded lazily in each page) ──────────
    m_statusPage  = new StatusPage(m_stack);
    m_personaPage = new PersonaPage(m_stack);
    m_ssidPage    = new SsidPage(m_stack);
    m_doctorPage  = new DoctorPage(m_stack);

    m_stack->addWidget(m_statusPage);   // index 0
    m_stack->addWidget(m_personaPage);  // index 1
    m_stack->addWidget(m_ssidPage);     // index 2
    m_stack->addWidget(m_doctorPage);   // index 3

    statusBar()->showMessage(QStringLiteral("Proteus GUI — skeleton"));
}

void MainWindow::setupNav()
{
    m_nav->addItem(QStringLiteral("Status"));
    m_nav->addItem(QStringLiteral("Personas"));
    m_nav->addItem(QStringLiteral("Per-SSID Rules"));
    m_nav->addItem(QStringLiteral("Doctor / Tools"));

    connect(m_nav, &QListWidget::currentRowChanged,
            this,  &MainWindow::onNavChanged);
}

void MainWindow::onNavChanged(int row)
{
    if (row < 0 || row >= m_stack->count()) return;
    m_stack->setCurrentIndex(row);

    // Trigger a data refresh on the now-visible page.
    // Each page exposes a `refresh()` slot for this purpose.
    switch (row) {
    case 0: m_statusPage->refresh();  break;
    case 1: m_personaPage->refresh(); break;
    case 2: m_ssidPage->refresh();    break;
    case 3: m_doctorPage->refresh();  break;
    default: break;
    }
}

} // namespace proteus
