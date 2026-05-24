// SPDX-License-Identifier: GPL-3.0-or-later
//
// ProteusRunner.cpp — implementation of the read-only QProcess wrapper.
//
// Design notes:
//
//  • Every read call appends "--json" so the CLI emits machine-readable
//    output rather than the ANSI-coloured human table.
//
//  • stdout is captured via QProcess::readAllStandardOutput().  stderr is
//    intentionally discarded (it carries human-facing tracing/logging that
//    is irrelevant to the Qt front-end; the front-end learns about errors
//    via the JSON body or the exit code).
//
//  • We never pass --yes here.  That flag is the contract boundary between
//    read and write; it lives exclusively in ProteusMutate.cpp.
//
//  • On timeout we kill() the child and surface a RunResult{ok=false}.

#include "ProteusRunner.h"

#include <QJsonParseError>
#include <QtConcurrent/QtConcurrent>

namespace Proteus {

// ── Construction ─────────────────────────────────────────────────────────────

ProteusRunner::ProteusRunner(QObject *parent)
    : QObject(parent)
    , m_binaryPath(QStringLiteral("proteus"))
{}

ProteusRunner::ProteusRunner(const QString &binaryPath, QObject *parent)
    : QObject(parent)
    , m_binaryPath(binaryPath)
{}

// ── Configuration ─────────────────────────────────────────────────────────────

void ProteusRunner::setTimeoutMs(int ms) { m_timeoutMs = ms; }
int  ProteusRunner::timeoutMs()    const { return m_timeoutMs; }
QString ProteusRunner::binaryPath() const { return m_binaryPath; }

// ── Named read helpers ────────────────────────────────────────────────────────

RunResult ProteusRunner::status()     const { return runAndParse({QStringLiteral("status"),      QStringLiteral("--json")}); }
RunResult ProteusRunner::session()    const { return runAndParse({QStringLiteral("session"),     QStringLiteral("--json")}); }
RunResult ProteusRunner::current()    const { return runAndParse({QStringLiteral("current"),     QStringLiteral("--json")}); }
RunResult ProteusRunner::original()   const { return runAndParse({QStringLiteral("original"),    QStringLiteral("--json")}); }
RunResult ProteusRunner::showConfig() const { return runAndParse({QStringLiteral("show-config"), QStringLiteral("--json")}); }

// ── Generic query ─────────────────────────────────────────────────────────────

RunResult ProteusRunner::query(const QStringList &args) const
{
    // Ensure --json is always present even if the caller forgot.
    QStringList fullArgs = args;
    if (!fullArgs.contains(QStringLiteral("--json")))
        fullArgs.append(QStringLiteral("--json"));
    return runAndParse(fullArgs);
}

// ── Async query ───────────────────────────────────────────────────────────────

void ProteusRunner::queryAsync(const QStringList &args)
{
    // Run the blocking CLI call in a Qt thread-pool worker so the GUI thread
    // is never stalled.  QFuture::result() is delivered back to the calling
    // thread via a QFutureWatcher connected to queryFinished().
    //
    // We capture m_binaryPath and m_timeoutMs by value so the lambda is
    // safe to execute on the worker thread after this QObject may have been
    // moved or destroyed.  The watcher is parented to `this`; if `this` is
    // deleted before the future completes the watcher is destroyed and the
    // finished() connection is safely disconnected — queryFinished() will
    // NOT be emitted after destruction.
    const QString   binary  = m_binaryPath;
    const int       timeout = m_timeoutMs;
    QStringList     fullArgs = args;
    if (!fullArgs.contains(QStringLiteral("--json")))
        fullArgs.append(QStringLiteral("--json"));

    auto *watcher = new QFutureWatcher<RunResult>(this);
    connect(watcher, &QFutureWatcher<RunResult>::finished,
            this, [this, watcher]() {
                emit queryFinished(watcher->result());
                watcher->deleteLater();
            });

    // Worker lambda: executed on a thread-pool thread.  Must not touch Qt
    // GUI objects — QProcess is safe to use from non-GUI threads.
    watcher->setFuture(QtConcurrent::run([binary, timeout, fullArgs]() -> RunResult {
        RunResult result;
        result.ok       = false;
        result.exitCode = -1;

        QProcess proc;
        proc.setProcessChannelMode(QProcess::SeparateChannels);
        proc.start(binary, fullArgs);

        if (!proc.waitForStarted(timeout)) {
            result.errorString = QStringLiteral("Failed to start '%1': %2")
                .arg(binary, proc.errorString());
            return result;
        }
        if (!proc.waitForFinished(timeout)) {
            proc.kill();
            result.errorString = QStringLiteral("'%1' timed out after %2 ms")
                .arg(binary).arg(timeout);
            return result;
        }

        result.exitCode = proc.exitCode();
        const QByteArray raw = proc.readAllStandardOutput();

        QJsonParseError parseErr;
        result.doc = QJsonDocument::fromJson(raw, &parseErr);
        if (parseErr.error != QJsonParseError::NoError) {
            result.errorString = QStringLiteral("JSON parse error: %1 (offset %2)")
                .arg(parseErr.errorString()).arg(parseErr.offset);
            return result;
        }

        result.ok = (result.exitCode == 0) && !result.doc.isNull();
        if (!result.ok && result.errorString.isEmpty())
            result.errorString = QStringLiteral("proteus exited with code %1")
                .arg(result.exitCode);
        return result;
    }));
}

// ── Core implementation ───────────────────────────────────────────────────────

RunResult ProteusRunner::runAndParse(const QStringList &fullArgs) const
{
    RunResult result;
    result.ok       = false;
    result.exitCode = -1;

    QProcess proc;
    // Discard stderr — it carries human-readable tracing, not JSON.
    proc.setProcessChannelMode(QProcess::SeparateChannels);
    proc.start(m_binaryPath, fullArgs);

    if (!proc.waitForStarted(m_timeoutMs)) {
        result.errorString = QStringLiteral("Failed to start '%1': %2")
            .arg(m_binaryPath, proc.errorString());
        return result;
    }

    if (!proc.waitForFinished(m_timeoutMs)) {
        proc.kill();
        result.errorString = QStringLiteral("'%1' timed out after %2 ms")
            .arg(m_binaryPath)
            .arg(m_timeoutMs);
        return result;
    }

    result.exitCode = proc.exitCode();

    const QByteArray raw = proc.readAllStandardOutput();

    QJsonParseError parseErr;
    result.doc = QJsonDocument::fromJson(raw, &parseErr);

    if (parseErr.error != QJsonParseError::NoError) {
        result.errorString = QStringLiteral("JSON parse error: %1 (offset %2)")
            .arg(parseErr.errorString())
            .arg(parseErr.offset);
        return result;
    }

    // Non-zero exit is treated as a soft failure — the doc may still contain
    // useful diagnostic fields (e.g. a {"error": "..."} body).
    result.ok = (result.exitCode == 0) && !result.doc.isNull();
    if (!result.ok && result.errorString.isEmpty()) {
        result.errorString = QStringLiteral("proteus exited with code %1")
            .arg(result.exitCode);
    }

    return result;
}

} // namespace Proteus
