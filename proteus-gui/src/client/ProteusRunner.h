// SPDX-License-Identifier: GPL-3.0-or-later
//
// ProteusRunner.h — Local QProcess wrapper for read-only `proteus … --json`.
//
// TODO: replace this file with proteus-client once that shared library unit
// has merged.  The replacement contract:
//   - Same ProteusRunner class name and public API surface.
//   - ProteusRunner becomes a thin wrapper over proteus_client::Reader.
//   - No change required in any page (StatusPage, PersonaPage, …).
//
// Architecture
// ────────────
// ProteusRunner executes `proteus <args> --json` as the CURRENT user (no
// elevation).  All mutations must go through PkexecRunner instead.
//
// Thread model: all calls are SYNCHRONOUS and must be made from a non-UI
// thread or a background QThread.  Pages queue requests and receive results
// via Qt signals — see each Page's constructor for the thread setup.
//
// Security note: the `proteus` binary is found via an explicit absolute path
// (/usr/bin/proteus) to match the Rust binary's own PATH-reset policy.  We
// do NOT inherit the ambient $PATH for the same reason main.rs resets it to
// a known-good list on startup.

#pragma once

#include <QByteArray>
#include <QJsonDocument>
#include <QObject>
#include <QStringList>

namespace proteus {

/// Result of a read command.  Either holds parsed JSON or an error string.
struct RunResult {
    bool        ok;
    QJsonDocument json;  ///< valid iff ok == true
    QString     error;  ///< human-readable, iff ok == false
};

/// Synchronous, read-only runner.  Spawns `proteus <args> --json`.
///
/// Example:
/// ```cpp
/// auto r = ProteusRunner::run({"status"});
/// if (r.ok) { /* use r.json */ }
/// ```
class ProteusRunner : public QObject {
    Q_OBJECT
public:
    explicit ProteusRunner(QObject *parent = nullptr);

    /// Run `proteus <args> --json` and return the result.
    /// `args` must NOT contain `--json`; the runner appends it.
    /// Blocks the calling thread for up to `timeoutMs` milliseconds.
    static RunResult run(const QStringList &args, int timeoutMs = 5000);

    /// Convenience: run `proteus status --json`.
    static RunResult status();

    /// Convenience: run `proteus persona list --json`.
    static RunResult personaList();

    /// Convenience: run `proteus persona show <id> --json`.
    static RunResult personaShow(const QString &id);

    /// Convenience: run `proteus ssid list --json`.
    static RunResult ssidList();

    /// Convenience: run `proteus ssid show <ssid> --json`.
    static RunResult ssidShow(const QString &ssid);

    /// Convenience: run `proteus doctor --json`.
    static RunResult doctor();

    /// Convenience: run `proteus diff --json`.
    static RunResult diff();

    /// Convenience: run `proteus dry-run <inner> --json`.
    ///
    /// `inner` must be one of the dry-run sub-commands: apply, revert, rotate,
    /// reset, uninstall, pin, hostname, bluetooth.  The default is "apply" (shows
    /// what `proteus apply` would do without executing it).
    ///
    /// NOTE: `--json` is the inner flag consumed by dry-run's own parser, not the
    /// outer --json.  ProteusRunner::run() appends it; dry-run's trailing_var_arg
    /// passes it through to InnerArgs which understands it.
    static RunResult dryRun(const QString &inner = QStringLiteral("apply"));

private:
    /// Absolute path to the proteus binary.  Never falls back to $PATH.
    static const char *s_proteusBin;
};

} // namespace proteus
