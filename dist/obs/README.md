# openSUSE Build Service (OBS) publishing

Proteus is published to [openSUSE Build Service](https://build.opensuse.org)
which builds packages for Ubuntu, Debian, Fedora, openSUSE, and a long
tail of other distros in one place. Unlike Copr and Launchpad PPA, OBS
**pulls from GitHub itself** via the `_service` mechanism — no GitHub
Actions integration needed.

Users install via the OBS-hosted apt/dnf/zypper repositories. Per-distro
install lines are shown at:
`https://software.opensuse.org/download.html?package=proteus&project=home:<obs-user>`

## One-time setup

### 1. Create an OBS account

<https://build.opensuse.org> → "Sign Up" (or log in with an existing
openSUSE / GitHub account). OBS auto-creates a personal namespace at
`home:<your-username>` (e.g. `home:kit3713`).

### 2. Create the package

Web UI:

1. Navigate to your namespace: `https://build.opensuse.org/project/show/home:<obs-user>`.
2. "Create Package" → name `proteus`, fill description.
3. Save.

### 3. Upload the source files

On the new package page:

1. "Upload File" → upload `dist/obs/_service` (from this repo) as `_service`.
2. "Upload File" → upload `dist/rpm/proteus.spec` as `proteus.spec`.
   (OBS uses the same spec file as Fedora/Copr; the version field gets
   overwritten by the `set_version` service.)

For Debian/Ubuntu builds, also upload the `dist/debian/` directory
contents as a tarball or individually (OBS will use them when building
for `.deb` targets). The `_service` already fetches the full source
tree which includes `dist/debian/`, so OBS-builtin Debian recipes work
without extra uploads.

### 4. Enable build targets

On the package page → "Repositories" → "Add from a project":

Recommended starter set:
- `openSUSE:Factory` (rolling)
- `openSUSE:Leap:15.6`
- `Fedora:43`, `Fedora:Rawhide`
- `Debian:13` (or current stable)
- `Ubuntu:24.04`, `Ubuntu:22.04`

Save. OBS schedules the first build immediately.

### 5. (Optional) Wire the GitHub → OBS trigger

OBS polls GitHub daily by default (cadence configured in the `_service`
file's `mode="auto"`). For instant rebuilds on tag push, generate a
trigger token in OBS:

1. Package page → "Configuration" → "Web Hooks" → "Generate Token".
2. Copy the token URL (looks like
   `https://api.opensuse.org/trigger/runservice?project=home:kit3713&package=proteus&token=ABCDEF...`).
3. In GitHub repo Settings → Webhooks → Add webhook:
   - Payload URL: the OBS trigger URL
   - Content type: `application/json`
   - Trigger on: "Just the *push* event" (filters to `v*` tags below)
   - Active: yes

Now every tag push hits OBS within seconds; OBS re-runs `_service`,
fetches the fresh tag, and rebuilds.

### 6. (Optional) Publish a software.opensuse.org install widget

Once a few builds are green, your package shows up at
`https://software.opensuse.org/package/proteus` automatically.
Linking to that page in your README is the standard "install on
openSUSE / Ubuntu / Debian / Fedora" path for OBS-hosted projects.

## Local OBS development (osc CLI)

For iterating on `_service` or spec changes without going through the
web UI:

```bash
sudo dnf install osc       # or: sudo zypper install osc
osc co home:kit3713/proteus
cd home:kit3713/proteus
# edit _service or proteus.spec
osc ci -m "rev _service for tag pattern fix"
```

`osc` commits trigger a rebuild same as web-UI uploads.

## Notes for the maintainer

- **No GitHub Actions involvement.** OBS owns the build trigger; the
  GitHub release workflow does not touch OBS. This is intentional —
  the OBS build farm has access controls separate from GitHub, and
  pushing the trigger out of GHA simplifies the secrets surface.
- **One spec, many distros.** `dist/rpm/proteus.spec` is the single
  source of truth. OBS uses it for every `.rpm`-producing chroot.
  Debian/Ubuntu builds use the same `dist/debian/` files OBS finds in
  the source tree.
- **First-build debugging.** OBS surfaces build logs per-target; for
  each red target, click into the log to see the exact step that
  failed. Common first-time failures: missing `BuildRequires`, distro
  rust too old (mirror the rustup bootstrap from `.github/workflows/release.yml`
  in the spec for older chroots).
- **Daily polling.** Without the webhook trigger, OBS picks up new
  tags within ~24 h. The webhook is a nice-to-have, not essential.
