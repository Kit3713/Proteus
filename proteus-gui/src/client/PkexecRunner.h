// SPDX-License-Identifier: GPL-3.0-or-later
//
// PkexecRunner.h — Elevation helper for mutating `proteus` commands.
//
// All mutations (apply, revert, persona use/clear, ssid set/clear) MUST go
// through this class.  It wraps:
//   pkexec proteus <subcommand> --yes [extra args]
//
// The `--yes` flag bypasses the interactive confirmation prompt so that the
// GUI's own confirmation dialogs (QMessageBox) serve as the user-facing gate.
// A mutation must only be invoked AFTER the user has confirmed in the GUI.
//
// TODO: replace with proteus-client::ElevatedRunner once that unit merges.
//
// Security notes
// ──────────────
// 1. pkexec looks up the proteus binary by absolute path (/usr/bin/proteus),
//    not by name — safe against PATH manipulation by the desktop session.
// 2. Arguments are passed as a QStringList (never shell-interpolated).
// 3. This class must NOT be called from the UI thread.  Use QFuture or
//    QThread.  Each mutation method is synchronous/blocking.
// 4. The return value carries the exit code.  Non-zero means the mutation
//    failed (or was cancelled at the pkexec authorization dialog).

#pragma once

#include <QObject>
#include <QStringList>

namespace proteus {

struct MutateResult {
    bool    ok;
    int     exitCode;
    QString output;   ///< combined stdout + stderr on failure (for error dialogs)
};

class PkexecRunner : public QObject {
    Q_OBJECT
public:
    explicit PkexecRunner(QObject *parent = nullptr);

    // ── Mutations (all require confirmed user intent before calling) ──────────

    /// pkexec proteus apply --yes
    static MutateResult apply(int timeoutMs = 30000);

    /// pkexec proteus revert --yes
    static MutateResult revert(int timeoutMs = 30000);

    /// pkexec proteus persona use <id> --yes
    static MutateResult personaUse(const QString &id, int timeoutMs = 15000);

    /// pkexec proteus persona clear --yes
    static MutateResult personaClear(int timeoutMs = 15000);

    /// pkexec proteus persona random --yes
    static MutateResult personaRandom(int timeoutMs = 15000);

    /// pkexec proteus ssid set <ssid> <key> <value> --yes
    static MutateResult ssidSet(const QString &ssid,
                                const QString &key,
                                const QString &value,
                                int timeoutMs = 15000);

    /// pkexec proteus ssid clear <ssid> --yes
    static MutateResult ssidClear(const QString &ssid, int timeoutMs = 15000);

private:
    /// Internal helper: pkexec /usr/bin/proteus <args> --yes
    static MutateResult elevate(const QStringList &args, int timeoutMs);

    static const char *s_pkexecBin;
    static const char *s_proteusBin;
};

} // namespace proteus
