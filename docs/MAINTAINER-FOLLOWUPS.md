# Maintainer follow-ups

Operational follow-ups that need maintainer auth — branch deletions, tag
pushes, GitHub release drafting. Update this file when a new release is
queued. Delete sections once the action has been performed.

## v0.4.0-beta1 release

`Cargo.toml` is at `0.4.0-beta1`; `CHANGELOG.md` `[0.4.0-beta1]` section
documents the cycle. Once `main` is green:

```sh
git tag -a v0.4.0-beta1 -m "v0.4.0-beta1 — bug + vulnerability hunt"
git push origin v0.4.0-beta1
```

The `release.yml` workflow then builds the binaries (x86_64, aarch64),
produces RPM / .deb / Arch packages, and creates a draft GitHub release.
Review the artifact set, confirm the stripped binary is under the size
cap, and publish.

## v0.3 tag push (deferred from v0.3.1-alpha)

Both v0.3 tags exist on the local clone but the original auth context
could not push them. They are ancestors of `main`. To push:

```sh
git push origin v0.3.0-alpha v0.3.1-alpha
```

If the local clone has lost them, recreate from the `[0.3.1-alpha]` and
`[0.3.0-alpha]` markers in `CHANGELOG.md` (the commits matching those
release notes are the targets). These are pre-release alphas; flag the
release as pre-release in the GitHub UI.

## Stale `claude/*` branches

Five `claude/*` branches on the remote are leftover from prior automated
sessions. Their tips are ancestors of `main` through merged work. To
delete:

```sh
git push origin \
  --delete claude/build-roadmap-features-oh5wK \
  claude/code-review-and-profile-b4C6L \
  claude/roadmap-backend-stealth-0cgKt \
  claude/security-audit-sjnXX
```

Run `git log --oneline main..origin/<branch>` first if you want to
confirm there's nothing unmerged before deleting.

## Removing this file

When every section above has been performed, delete this file:

```sh
git rm docs/MAINTAINER-FOLLOWUPS.md
git commit -m "chore: drop maintainer follow-ups note"
git push origin main
```
