# Copr publishing

Proteus is published to [Fedora Copr](https://copr.fedorainfracloud.org/)
via the `publish-copr` job in `.github/workflows/release.yml`. Each tag
push (`v*`) submits the SRPM built by `build-rpm` to a Copr project,
which fans it out across every chroot you've enabled (Fedora 42/43/rawhide,
EPEL 9, openSUSE Leap, etc.).

Users then install with:

```bash
sudo dnf copr enable kit3713/proteus
sudo dnf install proteus
```

## One-time setup

### 1. Create a Fedora Account (FAS)

If you don't already have one: <https://accounts.fedoraproject.org>.

### 2. Create the Copr project

1. Log into <https://copr.fedorainfracloud.org> with FAS.
2. "New Project" → name it `proteus`. (Owner defaults to your FAS
   username; the workflow expects `<owner>/proteus`, e.g. `kit3713/proteus`.)
3. Description: a one-liner pointing at the GitHub repo + the project
   short description from `Cargo.toml`.
4. Pick chroots. Suggested minimum:
   - `fedora-42-x86_64`, `fedora-42-aarch64`
   - `fedora-43-x86_64`, `fedora-43-aarch64`
   - `fedora-rawhide-x86_64`, `fedora-rawhide-aarch64`
   - `epel-9-x86_64`, `epel-9-aarch64` (optional — EL9 backports)
5. Leave "Build Options" defaults. Save.

### 3. Get an API token

1. Go to <https://copr.fedorainfracloud.org/api/>.
2. The page shows a pre-rendered config file with `login = ...`,
   `username = ...`, and `token = ...` fields. Copy those three
   values out — you'll paste them as GitHub secrets, not as a config
   file in the repo.

### 4. Add four GitHub Actions config values

Settings → Secrets and variables → Actions. Split across the two tabs
so non-sensitive values are visible in logs (helpful for debugging)
and the auth bits stay encrypted:

**Secrets tab** → New repository secret:

| Secret name      | Value                                                                  |
| ---------------- | ---------------------------------------------------------------------- |
| `COPR_LOGIN`     | the `login` line from the API page (opaque auth string)                |
| `COPR_TOKEN`     | the `token` line from the API page (opaque auth string)                |

**Variables tab** → New repository variable:

| Variable name    | Value                                                                  |
| ---------------- | ---------------------------------------------------------------------- |
| `COPR_USERNAME`  | your FAS username (typically same as Copr owner)                       |
| `COPR_PROJECT`   | `<owner>/<project>` — e.g. `kit3713/proteus`                           |

The `publish-copr` job no-ops gracefully when `COPR_TOKEN` is unset, so
forks of this repo don't see a red workflow.

## Per-release behaviour

After the four secrets exist, every `git push origin v<version>` tag
triggers `.github/workflows/release.yml`, which builds the SRPM under
`build-rpm`, then `publish-copr` downloads the SRPM artifact and runs
`copr-cli build $COPR_PROJECT proteus-<version>-1.fc43.src.rpm`.

Copr then schedules per-chroot builds. Status is visible at
`https://copr.fedorainfracloud.org/coprs/<owner>/proteus/builds/`. A
typical Fedora-only build takes ~3–5 minutes per chroot; EPEL and
openSUSE chroots add another ~5 min each.

## Manual / one-off publish (no GitHub Actions)

If the workflow is broken or you want to publish out-of-band:

```bash
sudo dnf install copr-cli
# Drop your login/username/token from https://copr.fedorainfracloud.org/api/
# into ~/.config/copr (chmod 600). Then:
copr-cli build kit3713/proteus path/to/proteus-1.0.0-1.fc43.src.rpm
```

The SRPM lives on the GitHub release page under each tagged release
(`proteus-<version>-1.fc43.src.rpm`); download it and feed it to
`copr-cli`.

## Notes for the maintainer

- `dist/rpm/proteus.spec` is the canonical spec. Copr re-builds against
  it for every chroot; no per-distro patching needed.
- The spec's `BuildRequires: rust >= 1.85` may fail on older EPEL
  chroots where the system rust is pre-1.85. Either drop EPEL from
  the chroot list, or add a Rust-toolchain-from-rustup fallback step
  to the spec (the GitHub workflow already handles this via the
  pinned `rust-toolchain.toml`).
- Copr's webhook integration can also auto-trigger on push, but the
  current setup (push the SRPM the release workflow already built)
  ensures bit-identical SRPMs land in Copr and on the GitHub Release —
  helpful for reproducible-build verifiers.
