// SPDX-License-Identifier: GPL-3.0-or-later
//
// JournaldTail.cpp — STUB implementation.
//
// Nothing is connected to the journal yet.  The class registers its
// metatype so that ProteusEvent can travel across queued connections once
// the real implementation lands.
//
// See JournaldTail.h for the full implementation plan.

#include "JournaldTail.h"

namespace Proteus {

JournaldTail::JournaldTail(QObject *parent)
    : QObject(parent)
{
    // Register the event struct for queued signal/slot delivery.
    qRegisterMetaType<ProteusEvent>("Proteus::ProteusEvent");

    // TODO(roadmap-1.3.x): configure m_pollTimer interval and connect its
    // timeout() to a slot that calls sd_journal_next() in a loop, emitting
    // eventReceived() for each new entry.
}

JournaldTail::~JournaldTail()
{
    stop();
}

void JournaldTail::start()
{
    if (m_running)
        return;

    // TODO(roadmap-1.3.x): open the journal with sd_journal_open(), add the
    // SYSLOG_IDENTIFIER=proteus match, seek to the tail, and arm the
    // QSocketNotifier on sd_journal_get_fd().  For now, do nothing.
    m_running = true;
}

void JournaldTail::stop()
{
    if (!m_running)
        return;

    m_pollTimer.stop();

    // TODO(roadmap-1.3.x): call sd_journal_close() here.

    m_running = false;
}

bool JournaldTail::isRunning() const
{
    return m_running;
}

} // namespace Proteus
