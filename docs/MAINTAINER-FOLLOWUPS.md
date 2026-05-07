# Maintainer follow-ups — v0.3.1-alpha release

The v0.3.1-alpha work is fully on `origin/main` (tip `71b575b`). The
following operations need maintainer auth — the auth context that
landed the code commits has read-only access to non-`main` refs and
gets HTTP 403 / sideband disconnect on every attempt.

## 1. Push the release tags

Both tags exist locally and point at the right commits:

```sh
git tag --list | grep v0.3
# v0.3.0-alpha
# v0.3.1-alpha

git tag -l --format='%(refname:strip=2) -> %(*objectname:short)%(objectname:short)' v0.3.0-alpha v0.3.1-alpha
```

To push:

```sh
git push origin v0.3.0-alpha v0.3.1-alpha
```

If the tags have drifted on the local clone, recreate from main:

```sh
# v0.3.0-alpha points at the release commit (16adad7)
git tag -d v0.3.0-alpha
git tag -a v0.3.0-alpha 16adad7 -m "v0.3.0-alpha — Reach + Persona cycle substantial completion"

# v0.3.1-alpha points at the wrap-up commit (43d21e7)
git tag -d v0.3.1-alpha
git tag -a v0.3.1-alpha 43d21e7 -m "v0.3.1-alpha — final v0.3 wrap-up"

git push origin v0.3.0-alpha v0.3.1-alpha
```

## 2. Delete stale `claude/*` branches

Four `claude/*` branches on the remote are leftover from this and
prior automated sessions. The feature branch's tip is identical to
`main`; the others predate v0.2.7-alpha and are ancestors of `main`
through merged work.

```sh
git push origin \
  --delete claude/build-roadmap-features-oh5wK \
  claude/code-review-and-profile-b4C6L \
  claude/roadmap-backend-stealth-0cgKt \
  claude/security-audit-sjnXX
```

If any of those have unique commits the maintainer wants to keep,
`git log --oneline main..origin/<branch>` will show what's there
before deleting.

## 3. Cut the GitHub release

Once the tags are on the remote, draft the release notes from
`CHANGELOG.md` `[0.3.1-alpha]` and `[0.3.0-alpha]` sections. Both
are pre-release alphas, so set the "this is a pre-release" flag.

## 4. CI verification

After the tag push, the `release.yml` workflow fires. The 4.5 MB
binary cap was raised in v0.3.0-alpha to fit the v0.3 surface; the
v0.3.1-alpha stripped binary is 4,339,512 bytes locally. Verify the
CI job's stripped-size measurement comes in under 4,500,000.

## What landed in v0.3.1-alpha

See `CHANGELOG.md` for the full notes. Headline:

- Roadmap state: 4⏳ / 4🚧 / 80✅ on bullet count (~92% complete).
- 794 tests passing (was 421 at session start).
- 12 GitHub issues closed (#200-211).
- Cargo.toml: 0.1.0 → 0.3.1-alpha (catches up after the v0.2.x cycle
  was tracked only in CHANGELOG terms).

## Removing this file

Once the maintainer has handled steps 1-3, delete this file:

```sh
git rm docs/MAINTAINER-FOLLOWUPS.md
git commit -m "chore: drop maintainer follow-ups note (v0.3.1-alpha shipped)"
git push origin main
```
