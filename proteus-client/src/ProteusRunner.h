// SPDX-License-Identifier: GPL-3.0-or-later
//
// ProteusRunner.h — synchronous + async QProcess wrappers around:
//
//   proteus <subcommand> [args...] --json
//
// All methods are read-only.  Mutating operations live in ProteusMutate.h.
//
// The CLI binary path defaults to "proteus" (resolved via PATH) but can be
// overridden at construction time for testing / sandboxed environments.

#pragma once

#include <QJsonDocument>
#include <QObject>
#include <QProcess>
#include <QString>
#include <QStringList>
#include <QtConcurrent/QtConcurrent>

namespace Proteus {

/// Result returned by every synchronous runner call.
struct RunResult {
    bool      ok;          ///< true iff exit code == 0 and JSON parsed cleanly
    int       exitCode;    ///< raw QProcess exit code
    QJsonDocument doc;     ///< parsed JSON (invalid if ok == false)
    QString   errorString; ///< human-readable failure reason when ok == false
};

///
/// ProteusRunner
///
/// Thin wrapper that executes `proteus <subcommand> --json`, captures stdout,
/// and parses the response with QJsonDocument.
///
/// All public methods are const — they do not mutate system state.
/// For operations that require privilege (apply, rotate, kill, …) see
/// ProteusMutate.
///
class ProteusRunner : public QObject {
    Q_OBJECT

public:
    /// Construct a runner that resolves the CLI via PATH.
    explicit ProteusRunner(QObject *parent = nullptr);

    /// Construct a runner pointing at an explicit binary path (useful in
    /// unit-test harnesses where the real binary is a mock script).
    explicit ProteusRunner(const QString &binaryPath, QObject *parent = nullptr);

    // ── Synchronous calls (block until QProcess exits) ─────────────────────

    /// `proteus status --json`
    RunResult status() const;

    /// `proteus session --json`
    RunResult session() const;

    /// `proteus current --json`
    RunResult current() const;

    /// `proteus original --json`
    RunResult original() const;

    /// `proteus show-config --json`
    RunResult showConfig() const;

    /// Generic: execute any subcommand with --json and wait for result.
    /// Caller is responsible for ensuring the subcommand is read-only.
    RunResult query(const QStringList &args) const;

    // ── Configuration ──────────────────────────────────────────────────────

    /// Timeout in milliseconds to wait for the CLI process (default: 8000 ms).
    void   setTimeoutMs(int ms);
    int    timeoutMs() const;

    QString binaryPath() const;

signals:
    /// Emitted when an async query completes (see queryAsync).
    void queryFinished(const RunResult &result);

public slots:
    /// Async variant of query() — runs in a thread-pool worker via
    /// QtConcurrent::run so it never blocks the GUI thread.  Returns
    /// immediately; result is delivered via the queryFinished() signal
    /// from the GUI thread.
    void queryAsync(const QStringList &args);

private:
    /// Execute binary + args, wait up to m_timeoutMs, parse stdout as JSON.
    RunResult runAndParse(const QStringList &fullArgs) const;

    QString m_binaryPath;
    int     m_timeoutMs = 8000;
};

} // namespace Proteus
