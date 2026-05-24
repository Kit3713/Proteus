// SPDX-License-Identifier: GPL-3.0-or-later
//
// ProteusMutate.cpp — pkexec-based mutate wrappers.
//
// Design notes:
//
//  • Every mutating action goes through pkexec so the caller (tray / GUI)
//    never needs to be running as root.  Polkit shows its own authentication
//    dialog; the front-end just launches and waits for finished().
//
//  • We always append --yes so the CLI does not prompt interactively —
//    confirmation is implied by the user having authenticated through polkit.
//
//  • The QProcess object is heap-allocated with `this` as parent so it is
//    automatically cleaned up when the ProteusMutate instance is destroyed.
//    A new QProcess is created for each launch so concurrent actions are
//    isolated (each has its own exit code / stdout).
//
//  • Stdout / stderr of the child process are discarded here; the tray
//    should re-query via ProteusRunner after the finished() signal to
//    refresh displayed state.
//
// NOTE: No pkexec / proteus process is spawned during the scaffolding build.
//       These code paths exist for the actual feature implementation.

#include "ProteusMutate.h"

namespace Proteus {

ProteusMutate::ProteusMutate(QObject *parent)
    : QObject(parent)
{}

// ── Configuration ─────────────────────────────────────────────────────────────

void ProteusMutate::setPkexecPath(const QString &path) { m_pkexecPath = path; }
void ProteusMutate::setBinaryPath(const QString &path) { m_binaryPath = path; }

// ── Named mutate helpers ──────────────────────────────────────────────────────

MutateStatus ProteusMutate::rotate() { return launch(QStringLiteral("rotate")); }
MutateStatus ProteusMutate::kill()   { return launch(QStringLiteral("kill"));   }
MutateStatus ProteusMutate::apply()  { return launch(QStringLiteral("apply"));  }
MutateStatus ProteusMutate::revert() { return launch(QStringLiteral("revert")); }

// ── argv builder ──────────────────────────────────────────────────────────────

QStringList ProteusMutate::buildArgv(const QString &subcommand,
                                     const QStringList &extraArgs) const
{
    // Full argv:  pkexec  proteus  <subcommand>  [extraArgs...]  --yes
    QStringList argv;
    argv << m_binaryPath << subcommand;
    argv << extraArgs;
    if (!argv.contains(QStringLiteral("--yes")))
        argv << QStringLiteral("--yes");
    return argv;
}

// ── Core launcher ─────────────────────────────────────────────────────────────

MutateStatus ProteusMutate::launch(const QString &subcommand,
                                   const QStringList &extraArgs)
{
    const QStringList argv = buildArgv(subcommand, extraArgs);

    // Each invocation gets its own QProcess parented to `this`; it will be
    // auto-deleted when ProteusMutate is destroyed (or earlier, via the
    // finished lambda below).
    auto *proc = new QProcess(this);
    // Discard stdout/stderr from pkexec — the polkit authentication dialog is
    // graphical (not stdio-based).  ForwardedChannels would pollute the tray
    // process's own stdout and break any parent that reads its output.
    proc->setProcessChannelMode(QProcess::SeparateChannels);

    // Clean up and surface the result when the child exits.
    connect(proc, qOverload<int, QProcess::ExitStatus>(&QProcess::finished),
            this, [this, proc, subcommand](int exitCode, QProcess::ExitStatus) {
                emit finished(subcommand, exitCode);
                proc->deleteLater();
            });

    // pkexec is the actual binary; its first argument is the real proteus path.
    proc->start(m_pkexecPath, argv);

    if (proc->state() == QProcess::NotRunning) {
        // start() failed synchronously — e.g. pkexec not in PATH.
        proc->deleteLater();
        return MutateStatus::Failed;
    }

    return MutateStatus::Launched;
}

} // namespace Proteus
