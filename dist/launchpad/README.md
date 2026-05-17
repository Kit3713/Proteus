# Launchpad PPA publishing

Proteus is published to a Launchpad PPA via the `publish-ppa` job in
`.github/workflows/release.yml`. Each tag push builds a Debian source
package per Ubuntu series, signs it with the maintainer's GPG key, and
`dput`s it. Launchpad's build farm produces `.deb` binaries.

Users then install with:

```bash
sudo add-apt-repository ppa:kit3713/proteus
sudo apt update
sudo apt install proteus
```

## One-time setup

### 1. Create a Launchpad account

<https://launchpad.net> → "Log in" → Ubuntu One SSO. Pick a username
(this becomes the `lp-user` segment of your PPA URL).

### 2. Sign the Ubuntu Code of Conduct

Required before you can own a PPA.

1. <https://launchpad.net/codeofconduct> → "I agree, sign it"
2. Launchpad asks for a GPG key fingerprint to sign with. If you don't
   have one yet, generate one in step 3 first and come back.

### 3. Generate (or pick) a GPG key + upload it

```bash
gpg --full-generate-key
# Choose:
#   1 (RSA and RSA), 4096 bits, 0 (key does not expire),
#   Real name: Kit Collver
#   Email:     <the email you registered with Launchpad>
#   Passphrase: <strong; you'll paste it into a GitHub Secret>
```

Grab the long key ID:

```bash
gpg --list-secret-keys --keyid-format=LONG
# sec   rsa4096/ABCDEF0123456789 2026-05-17 [SC]
```

Export the public key for Launchpad:

```bash
gpg --send-keys --keyserver keyserver.ubuntu.com ABCDEF0123456789
```

Then in Launchpad: profile → "OpenPGP keys" → paste the fingerprint
(`gpg --fingerprint ABCDEF0123456789`). Launchpad will send a GPG-
encrypted email; decrypt it and click the confirmation link.

### 4. Create the PPA

1. Launchpad profile → "Create a new PPA"
2. URL: `proteus`
3. Display name: `Proteus — fingerprint scrubber`
4. Description: one-liner pointing at the GitHub repo.

Wait ~15 min for the PPA to initialize.

### 5. Add four GitHub Actions config values

Settings → Secrets and variables → Actions.

**Secrets tab** → New repository secret:

| Secret name                   | Value                                                                  |
| ----------------------------- | ---------------------------------------------------------------------- |
| `LAUNCHPAD_GPG_PRIVATE_KEY`   | ASCII-armored private key: `gpg --armor --export-secret-keys ABCDEF0123456789` |
| `LAUNCHPAD_GPG_PASSPHRASE`    | the passphrase you set when generating the key                         |

**Variables tab** → New repository variable:

| Variable name        | Value                                                          |
| -------------------- | -------------------------------------------------------------- |
| `LAUNCHPAD_PPA`      | `ppa:<lp-user>/proteus` — e.g. `ppa:kit3713/proteus`           |
| `LAUNCHPAD_SERIES`   | optional; comma-separated. Defaults to `noble`. Example: `noble,jammy,oracular` |

The `publish-ppa` job no-ops gracefully when `LAUNCHPAD_GPG_PRIVATE_KEY`
is unset, so forks of the repo don't see a red workflow.

## Per-release behaviour

After the four config values exist, every `git push origin v<version>`
tag triggers `.github/workflows/release.yml`. For each Ubuntu series in
`LAUNCHPAD_SERIES`, the workflow:

1. Materializes the debian/ layout from `dist/debian/`.
2. Bumps `debian/changelog` to `<version>-1~<series>1` (PPA disallows
   re-uploading the same source version; the `~<series>N` suffix
   disambiguates one source per target series).
3. Builds the source package with `dpkg-buildpackage -S -sa -d`.
4. Signs the resulting `.changes` with the imported GPG key.
5. `dput`s to the configured PPA.

Launchpad's build farm picks up each upload and produces binaries.
Status visible at `https://launchpad.net/~<lp-user>/+archive/ubuntu/proteus`.
Typical build time: 10–30 min per series after upload.

## Manual upload (no GitHub Actions)

If the workflow is broken or you want to publish out-of-band:

```bash
sudo apt install devscripts dput debhelper
# Clone Proteus, stage debian/, build the source package:
git clone https://github.com/Kit3713/Proteus.git
cd Proteus
cp -a dist/debian debian
debchange --newversion 1.0.0-1~noble1 --distribution noble --force-distribution "manual upload"
dpkg-buildpackage -S -sa
cd ..
debsign -k ABCDEF0123456789 proteus_1.0.0-1~noble1_source.changes
dput ppa:kit3713/proteus proteus_1.0.0-1~noble1_source.changes
```

## Notes for the maintainer

- **Series selection.** Default is `noble` (24.04 LTS). For each
  additional series you target, Launchpad will run a separate build —
  some older series may lack newer dependencies (Ubuntu 22.04 jammy
  ships rustc 1.75, but `dist/debian/control` floors at 1.85, so jammy
  builds need a rustup bootstrap step in `debian/rules`, or a separate
  toolchain). Start with `noble` only; add more as you debug each.
- **Key hygiene.** Use a dedicated GPG key for Launchpad. Don't reuse
  your code-signing or personal email key. The private key has to
  live in a GitHub secret, and that's a wider blast radius than a
  laptop-only key.
- **Per-series version suffix.** The `~<series>1` suffix means a
  v1.0.0 release lands as `1.0.0-1~noble1`, `1.0.0-1~jammy1`, etc. To
  re-upload (e.g. fix a packaging bug without bumping the upstream
  version), increment the tail to `~noble2`. Launchpad rejects exact
  duplicate uploads.
- **First upload reject is common.** Launchpad's reject reasons are
  emailed to the account; common first-time gotchas: Code of Conduct
  unsigned, GPG key not confirmed, debian/changelog distribution =
  `unstable` instead of a real Ubuntu series, source-format mismatch.
