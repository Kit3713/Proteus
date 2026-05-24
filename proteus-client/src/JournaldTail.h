// SPDX-License-Identifier: GPL-3.0-or-later
//
// JournaldTail.h — stub interface for streaming live proteus log events from
// systemd-journald.
//
// The interface is complete; the implementation is a TODO stub.
//
// Intended usage:
//
//   JournaldTail tail;
//   connect(&tail, &JournaldTail::eventReceived, myWidget, &MyWidget::onEvent);
//   tail.start();   // opens the journal and begins polling
//   ...
//   tail.stop();
//
// Implementation notes (for the eventual real impl):
//
//   Use QSocketNotifier on the fd returned by sd_journal_get_fd() (from
//   libsystemd) to receive POLLIN readiness notifications without blocking.
//   Filter with  SD_JOURNAL_MATCH_ADD("SYSLOG_IDENTIFIER=proteus").
//   Re-emit each field set as a ProteusEvent struct.
//
//   Alternatively, tail `journalctl -f -o json -t proteus` via a QProcess —
//   lower-privilege, no sd_journal linkage, but slightly higher latency.
//
// This file does NOT link against libsystemd in the current skeleton.

#pragma once

#include <QObject>
#include <QTimer>
#include <QString>
#include <QVariantMap>

namespace Proteus {

/// A single structured event as emitted by the proteus CLI to journald.
struct ProteusEvent {
    QString   timestamp;   ///< ISO-8601 string, from REALTIME_TIMESTAMP
    QString   level;       ///< "trace" | "debug" | "info" | "warn" | "error"
    QString   message;     ///< MESSAGE field (may be pre-redacted by the CLI)
    QVariantMap extra;     ///< any additional structured fields
};

///
/// JournaldTail
///
/// Opens the systemd journal and emits eventReceived for each new proteus log
/// entry.  This is a STUB — start() is a no-op and eventReceived is never
/// emitted.  See header comments for the implementation roadmap.
///
class JournaldTail : public QObject {
    Q_OBJECT

public:
    explicit JournaldTail(QObject *parent = nullptr);
    ~JournaldTail() override;

    /// Start tailing.  No-op in stub; will open the journal in real impl.
    void start();

    /// Stop tailing and release journal handle.
    void stop();

    bool isRunning() const;

signals:
    /// Emitted for each new log entry received from the journal.
    void eventReceived(const Proteus::ProteusEvent &event);

    /// Emitted when the journal connection is lost or an error occurs.
    void error(const QString &description);

private:
    // TODO(roadmap-1.3.x): Replace QTimer poll stub with sd_journal_get_fd() +
    // QSocketNotifier.  Requires linking against libsystemd; add:
    //   find_package(PkgConfig REQUIRED)
    //   pkg_check_modules(SYSTEMD REQUIRED libsystemd)
    //   target_link_libraries(proteus-client PUBLIC ${SYSTEMD_LIBRARIES})
    // to CMakeLists.txt, and add  BuildRequires: pkgconfig(libsystemd)  to
    // the RPM sub-package spec.

    bool   m_running = false;
    QTimer m_pollTimer; ///< Placeholder — unused in stub; drives real impl
};

} // namespace Proteus

// Make ProteusEvent usable in queued signal/slot connections across threads.
Q_DECLARE_METATYPE(Proteus::ProteusEvent)
