// SPDX-License-Identifier: GPL-3.0-or-later
//
// ProteusRunner.cpp — implementation.
//
// TODO: replace with proteus-client once available (see header).

#include "ProteusRunner.h"

#include <QProcess>
#include <QJsonParseError>

namespace proteus {

// Absolute path to the installed binary.  Must match the install prefix used
// by `cargo install` / the RPM.  We intentionally do NOT use QStandardPaths
// or $PATH here — the Rust binary itself resets $PATH to /usr/sbin:…:/usr/bin
// on startup for the same reason.
const char *ProteusRunner::s_proteusBin = "/usr/bin/proteus";

ProteusRunner::ProteusRunner(QObject *parent)
    : QObject(parent)
{
}

RunResult ProteusRunner::run(const QStringList &args, int timeoutMs)
{
    QProcess proc;
    proc.setProgram(QString::fromLatin1(s_proteusBin));

    // Always append --json so the output is machine-parseable.
    QStringList fullArgs = args;
    fullArgs << QStringLiteral("--json");

    proc.setArguments(fullArgs);

    // Inherit a clean, known-good environment.  We deliberately do NOT
    // inherit RUST_LOG from the ambient environment so the GUI doesn't
    // accidentally trigger verbose proteus logging that we'd try to JSON-parse.
    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    env.remove(QStringLiteral("RUST_LOG"));
    proc.setProcessEnvironment(env);

    proc.start();
    if (!proc.waitForStarted(timeoutMs)) {
        return RunResult{
            false,
            {},
            QStringLiteral("proteus did not start within %1 ms: %2")
                .arg(timeoutMs)
                .arg(proc.errorString())
        };
    }
    if (!proc.waitForFinished(timeoutMs)) {
        proc.kill();
        return RunResult{
            false,
            {},
            QStringLiteral("proteus timed out after %1 ms").arg(timeoutMs)
        };
    }

    if (proc.exitStatus() != QProcess::NormalExit || proc.exitCode() != 0) {
        // Use stderrStr, not 'stderr': on musl libc `stderr` is a macro
        // that expands to a FILE* expression, which would cause a compile error.
        QString stderrStr = QString::fromUtf8(proc.readAllStandardError()).trimmed();
        return RunResult{
            false,
            {},
            QStringLiteral("proteus exited %1: %2").arg(proc.exitCode()).arg(stderrStr)
        };
    }

    QByteArray out = proc.readAllStandardOutput();
    QJsonParseError parseErr;
    QJsonDocument doc = QJsonDocument::fromJson(out, &parseErr);
    if (doc.isNull()) {
        return RunResult{
            false,
            {},
            QStringLiteral("JSON parse error: %1").arg(parseErr.errorString())
        };
    }

    return RunResult{true, doc, {}};
}

// ── Convenience wrappers ──────────────────────────────────────────────────────
//
// Each wrapper mirrors exactly one `proteus <subcommand>` that has --json
// support today.  The set here is the faithful mirror of the CLI surface
// the GUI exposes — no extra commands, no scope drift.

RunResult ProteusRunner::status()
{
    return run({QStringLiteral("status")});
}

RunResult ProteusRunner::personaList()
{
    return run({QStringLiteral("persona"), QStringLiteral("list")});
}

RunResult ProteusRunner::personaShow(const QString &id)
{
    // id is validated by the CLI (persona id must match [A-Za-z0-9_-], ≤64 chars).
    // We pass it through unchanged; the CLI will reject invalid ids with a
    // non-zero exit code which RunResult::error captures.
    return run({QStringLiteral("persona"), QStringLiteral("show"), id});
}

RunResult ProteusRunner::ssidList()
{
    return run({QStringLiteral("ssid"), QStringLiteral("list")});
}

RunResult ProteusRunner::ssidShow(const QString &ssid)
{
    return run({QStringLiteral("ssid"), QStringLiteral("show"), ssid});
}

RunResult ProteusRunner::doctor()
{
    return run({QStringLiteral("doctor")});
}

RunResult ProteusRunner::diff()
{
    return run({QStringLiteral("diff")});
}

RunResult ProteusRunner::dryRun(const QString &inner)
{
    // `proteus dry-run` requires an inner subcommand (apply, revert, etc.).
    // run() appends --json which becomes part of the trailing_var_arg slice;
    // dry-run's inner InnerArgs parser understands --json as a flag before or
    // after the subcommand.  Without an inner subcommand, dry-run exits 64.
    return run({QStringLiteral("dry-run"), inner});
}

} // namespace proteus
