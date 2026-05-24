// SPDX-License-Identifier: GPL-3.0-or-later
//
// PkexecRunner.cpp — implementation.
//
// TODO: replace with proteus-client::ElevatedRunner once available (see header).
//
// IMPORTANT: This file contains CODE ONLY.  The pkexec paths are NOT exercised
// during the skeleton validation step.  Do NOT call any of these functions
// from test harnesses or CI — they require a live PolicyKit agent and a real
// proteus installation.

#include "PkexecRunner.h"

#include <QProcess>

namespace proteus {

const char *PkexecRunner::s_pkexecBin  = "/usr/bin/pkexec";
const char *PkexecRunner::s_proteusBin = "/usr/bin/proteus";

PkexecRunner::PkexecRunner(QObject *parent)
    : QObject(parent)
{
}

MutateResult PkexecRunner::elevate(const QStringList &args, int timeoutMs)
{
    QProcess proc;
    proc.setProgram(QString::fromLatin1(s_pkexecBin));

    // Build: pkexec /usr/bin/proteus <subcommand args> --yes
    QStringList fullArgs;
    fullArgs << QString::fromLatin1(s_proteusBin);
    fullArgs << args;
    fullArgs << QStringLiteral("--yes");

    proc.setArguments(fullArgs);

    // Restrict the environment to a minimal known-good set.
    // pkexec passes its own sanitized environment to the privileged child,
    // but we also sanitize what we hand to pkexec itself.
    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    env.remove(QStringLiteral("RUST_LOG"));
    proc.setProcessEnvironment(env);

    proc.start();
    if (!proc.waitForStarted(5000)) {
        return MutateResult{
            false,
            -1,
            QStringLiteral("pkexec did not start: %1").arg(proc.errorString())
        };
    }
    if (!proc.waitForFinished(timeoutMs)) {
        proc.kill();
        return MutateResult{false, -1, QStringLiteral("pkexec timed out")};
    }

    int code = proc.exitCode();
    bool ok   = (proc.exitStatus() == QProcess::NormalExit && code == 0);

    QString output;
    if (!ok) {
        output  = QString::fromUtf8(proc.readAllStandardOutput()).trimmed();
        QString err = QString::fromUtf8(proc.readAllStandardError()).trimmed();
        if (!err.isEmpty()) {
            if (!output.isEmpty()) output += QLatin1Char('\n');
            output += err;
        }
    }

    return MutateResult{ok, code, output};
}

// ── Public mutation methods ───────────────────────────────────────────────────

MutateResult PkexecRunner::apply(int timeoutMs)
{
    return elevate({QStringLiteral("apply")}, timeoutMs);
}

MutateResult PkexecRunner::revert(int timeoutMs)
{
    return elevate({QStringLiteral("revert")}, timeoutMs);
}

MutateResult PkexecRunner::personaUse(const QString &id, int timeoutMs)
{
    return elevate({QStringLiteral("persona"), QStringLiteral("use"), id}, timeoutMs);
}

MutateResult PkexecRunner::personaClear(int timeoutMs)
{
    return elevate({QStringLiteral("persona"), QStringLiteral("clear")}, timeoutMs);
}

MutateResult PkexecRunner::personaRandom(int timeoutMs)
{
    return elevate({QStringLiteral("persona"), QStringLiteral("random")}, timeoutMs);
}

MutateResult PkexecRunner::ssidSet(const QString &ssid,
                                   const QString &key,
                                   const QString &value,
                                   int timeoutMs)
{
    return elevate({QStringLiteral("ssid"), QStringLiteral("set"), ssid, key, value}, timeoutMs);
}

MutateResult PkexecRunner::ssidClear(const QString &ssid, int timeoutMs)
{
    return elevate({QStringLiteral("ssid"), QStringLiteral("clear"), ssid}, timeoutMs);
}

} // namespace proteus
