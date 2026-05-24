// SPDX-License-Identifier: GPL-3.0-or-later
//
// ProteusMutate.h — build and (asynchronously) launch pkexec invocations
// for every state-changing proteus subcommand.
//
// Architecture contract (from docs/ROADMAP.md):
//
//   Front-ends READ via: proteus <subcommand> --json
//   Front-ends WRITE via: pkexec proteus <subcommand> --yes
//
// This class covers the WRITE side.  It constructs the argv array and
// exposes it to callers; the actual QProcess::start() call happens on an
// explicit launch() invocation so callers can inspect/log the command first.
//
// IMPORTANT: None of the launch() methods are called during the build of
// this skeleton (Qt6 is absent on the build host).  The code is compiled
// but never executed here.

#pragma once

#include <QObject>
#include <QProcess>
#include <QString>
#include <QStringList>

namespace Proteus {

/// Status codes for a mutate operation launch.
enum class MutateStatus {
    Launched,   ///< QProcess started successfully; outcome is asynchronous.
    Failed,     ///< QProcess failed to start (binary not found, etc.).
    Refused,    ///< pkexec authentication was cancelled by the user.
};

///
/// ProteusMutate
///
/// Builds `pkexec proteus <subcommand> --yes` invocations and launches them
/// via QProcess.  The process runs asynchronously; callers connect to the
/// finished() signal to learn the outcome.
///
/// GUI thread safety: all public methods are safe to call from the GUI
/// thread.  QProcess emits finished() on the same thread.
///
class ProteusMutate : public QObject {
    Q_OBJECT

public:
    explicit ProteusMutate(QObject *parent = nullptr);

    // ── Mutate actions ─────────────────────────────────────────────────────

    /// `pkexec proteus rotate --yes`
    MutateStatus rotate();

    /// `pkexec proteus kill --yes`
    MutateStatus kill();

    /// `pkexec proteus apply --yes`
    MutateStatus apply();

    /// `pkexec proteus revert --yes`
    MutateStatus revert();

    // ── Configuration ──────────────────────────────────────────────────────

    /// Override the pkexec binary path (default: "pkexec" via PATH).
    void setPkexecPath(const QString &path);

    /// Override the proteus binary path (default: "proteus" via PATH).
    void setBinaryPath(const QString &path);

    /// Build (but do not launch) the argv for a given subcommand + extra args.
    /// Useful for logging/auditing before actual launch.
    QStringList buildArgv(const QString &subcommand,
                          const QStringList &extraArgs = {}) const;

signals:
    /// Emitted when the pkexec child exits.
    /// exitCode == 126 means the user cancelled polkit authentication.
    /// exitCode == 127 means the binary was not found.
    void finished(const QString &subcommand, int exitCode);

private:
    MutateStatus launch(const QString &subcommand, const QStringList &extraArgs = {});

    QString m_pkexecPath  = QStringLiteral("pkexec");
    QString m_binaryPath  = QStringLiteral("proteus");
};

} // namespace Proteus
