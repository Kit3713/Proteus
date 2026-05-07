# Maintainer follow-ups — v0.3.2-alpha release (final alpha)

`v0.3.2-alpha` is the **final alpha** of the v0.3 cycle. The next
cycle is `v0.4` beta — bug + vulnerability hunt only. See
[`BETA-INTAKE.md`](BETA-INTAKE.md) for the operational shape.

The work for v0.3.0-alpha, v0.3.1-alpha, and v0.3.2-alpha is fully on
the topic branch / `origin/main`. The following operations need
maintainer auth — the auth context that landed the code commits has
read-only access to non-`main` refs and gets HTTP 403 / sideband
disconnect on every attempt.

## 1. Push the release tags

The full v0.3 tag chain:

```sh
git tag --list | grep v0.3
# v0.3.0-alpha
# v0.3.1-alpha
# v0.3.2-alpha   <-- final alpha; closes the v0.3 cycle
```

To push:

```sh
git push origin v0.3.0-alpha v0.3.1-alpha v0.3.2-alpha
```

The v0.3.2-alpha tag does not exist yet on the topic branch — create
it after merging the cycle-close PR. From the merge commit on `main`:

```sh
git tag -a v0.3.2-alpha -m "v0.3.2-alpha — final alpha; v0.3 cycle close"
git push origin v0.3.2-alpha
```

If the older tags have drifted on the local clone, recreate from main:

```sh
# v0.3.0-alpha points at the release commit (16adad7)
git tag -d v0.3.0-alpha
git tag -a v0.3.0-alpha 16adad7 -m "v0.3.0-alpha — Reach + Persona cycle substantial completion"

# v0.3.1-alpha points at the wrap-up commit (43d21e7)
git tag -d v0.3.1-alpha
git tag -a v0.3.1-alpha 43d21e7 -m "v0.3.1-alpha — final v0.3 wrap-up"

git push origin v0.3.0-alpha v0.3.1-alpha v0.3.2-alpha
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
`CHANGELOG.md` `[0.3.2-alpha]`, `[0.3.1-alpha]`, and `[0.3.0-alpha]`
sections. All three are pre-release alphas, so set the "this is a
pre-release" flag.

The `v0.3.2-alpha` release is the **final alpha** and should call out
the v0.4 beta cycle in the release-notes preamble — link to
`docs/BETA-INTAKE.md` and `docs/ROADMAP-v0.4.md` so contributors land
on the right intake form.

## 4. CI verification

After the tag push, the `release.yml` workflow fires. The 4.5 MB
binary cap was raised in v0.3.0-alpha to fit the v0.3 surface; the
v0.3.1-alpha stripped binary measured 4,339,512 bytes. v0.3.2-alpha
adds the `crate::process` module + the JSON-to-YAML emitter (~10 KB
of new code total — no new deps), so the stripped binary will grow
slightly. Verify the CI job's stripped-size measurement still comes
in under 4,500,000.

## What landed in v0.3.2-alpha

See `CHANGELOG.md` for the full notes. Headline (final alpha — closes
the v0.3 cycle):

- Roadmap state: **83 ✅ / 1 💭** (Debian unstable submission, deferred —
  needs sponsor).
- **830 tests passing** (added 12 in this release: yaml emitter,
  parse_duration corner-case regressions, process module).
- `cargo clippy --all-targets` produces zero warnings.
- Closes M3 connection-up, M4a persona-aware NTP + nft, M4c
  `renew_on_apply` orchestrator wiring, M5 doctor next-steps, M6
  `--format yaml` (zero-dep emitter), M6 bypass-hardening pass
  (caught and fixed two real bugs in `per_ssid::parse_duration`),
  M6 wiki-hint sweep.

## What landed in v0.3.1-alpha

(historical, kept for tag-push reference)

- Roadmap state: 4⏳ / 4🚧 / 80✅ on bullet count (~92% complete).
- 794 tests passing.
- 12 GitHub issues closed (#200-211).
- Cargo.toml: 0.1.0 → 0.3.1-alpha (catches up after the v0.2.x cycle
  was tracked only in CHANGELOG terms).

## Removing this file

Once the maintainer has tagged + pushed `v0.3.2-alpha` and drafted
the GitHub release notes, delete this file:

```sh
git rm docs/MAINTAINER-FOLLOWUPS.md
git commit -m "chore: drop maintainer follow-ups note (v0.3.2-alpha shipped)"
git push origin main
```
