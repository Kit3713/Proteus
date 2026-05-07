# Runtime efficiency baseline

This doc records the measured cost of cold-start `proteus` invocations so future
regressions are detectable. The first round (May 2026) also delivered a small
optimization: a fast path in `logging::init` that skips installing the global
`tracing-subscriber` when nothing meaningful would be logged.

The user-visible cost the project cares about is **runtime**, not on-disk size:

> "i dont want the wiki preloaded, i want everything to run minimally as
> possible, that is much more important than binary size."

## What was measured

Three representative cold-start commands, repeated under valgrind/callgrind for
deterministic instruction counts and via interleaved wall-clock timing for
end-to-end latency:

- `proteus --help` — clap exits before our `cli::run` body. A pure
  process-startup baseline (linker, glibc init, clap command-tree build).
- `proteus show-defaults --json` — the cheapest path that *does* run our
  `cli::run` body (logging init, dispatch, `Config::default()` + JSON render).
- `proteus --version` — same as `--help` for our purposes; clap handles it
  before reaching our code.

`proteus version` is **not** a subcommand (clap rejects it with an
`unrecognized subcommand` error), so its instruction count is unrelated to the
init path and is not used for comparison.

The host: Fedora 43, glibc 2.42, kernel 6.19, x86\_64. Release profile is
`opt-level = "z"`, `lto = true`, `strip = true`, `panic = "abort"`,
`codegen-units = 1`. Binary is 3.68 MB stripped.

## Tools

- `valgrind --tool=callgrind` for instruction counts and per-symbol cost
  (deterministic, immune to noise; the gold standard).
- `/usr/bin/time -v` for peak RSS and minor page faults.
- A bash interleaved-timing script (50–500 paired runs, baseline / optimized
  alternating) for wall-clock medians.
- A separate `release-symbols` profile (`debug = "limited"`, `strip = false`,
  inherits release otherwise) for symbolic profiling. Not part of the shipped
  build; used only for measurement.

`strace` was unavailable here (`kernel.yama.ptrace_scope = 2`) so we relied on
callgrind's `--collect-systime=yes` for syscall counts when needed.

## Methodology gotchas

- **`JOURNAL_STREAM` is set in many shells under systemd-managed terminals.**
  When set, proteus picks the journald subscriber path and the fast path below
  is skipped — measurements look identical to baseline. Always run with
  `env -u JOURNAL_STREAM …` to capture the interactive shell case.
- **Binary path length affects clap's argv-parsing cost.** Running
  `/long/path/to/proteus …` measurably slowed clap's mkeymap on the optimized
  binary in early measurements until both binaries lived at the same depth
  (`/tmp/proteus-base` vs `/tmp/proteus-opt`). For comparisons, place the two
  binaries side-by-side under identical-length paths.
- **A single `/usr/bin/time -v` invocation reports peak RSS that depends on
  pagecache state.** The first run after a fresh build can show 4–5 MB; the
  steady-state median across 20–50 runs is ~970 kB for `--help`/`--version`
  /`show-defaults --json`. Use the median, not a single-shot reading.

## Baseline (commit at branch base)

Instruction counts, `JOURNAL_STREAM` unset:

| command                  | I instructions |
| ------------------------ | -------------- |
| `--help`                 | 2,876,662      |
| `--version`              | 2,876,662 \*   |
| `show-defaults --json`   | 2,741,808      |

\* `--help` and `--version` go through identical clap paths up to the exit; we
report a single number.

Steady-state RSS, `JOURNAL_STREAM` unset, 50 samples each:

| command                | RSS min | RSS median | RSS max |
| ---------------------- | ------- | ---------- | ------- |
| `--help`               | 940 kB  | 968 kB     | 996 kB  |
| `--version`            | 784 kB  | 968 kB     | 996 kB  |
| `show-defaults --json` | 784 kB  | 976 kB     | 996 kB  |

Wall-clock (200 paired runs interleaved against the optimized build at the same
filesystem depth):

| command                | baseline median |
| ---------------------- | --------------- |
| `--help`               | ~1.05 ms        |
| `--version`            | ~1.02 ms        |
| `show-defaults --json` | ~1.39 ms        |

## Where the cycles go (baseline)

From callgrind's per-function breakdown of `show-defaults --json` (the only
cheap command that actually runs our `cli::run` body):

| % of insns | site                                                            |
| ---------- | --------------------------------------------------------------- |
| ~50–55 %   | `clap_builder::*` building the command tree on every invocation |
| ~6 %       | glibc `_int_malloc`                                             |
| ~5 %       | linker `do_lookup_x`                                            |
| ~3 %       | linker `_dl_relocate_object_no_relro`                           |
| ~3 %       | `__memcpy_avx_unaligned_erms`                                   |
| ~1.7 %     | `tracing_subscriber::registry` + `sharded-slab` shard init      |
| <1 %       | `serde_json` output                                             |
| <1 %       | `Config::default()` + TOML/JSON serialization                   |

The dominant cost is **clap building the command tree** every run. clap-derive
re-allocates the entire command/argument graph regardless of which subcommand
is requested. Removing that would mean replacing clap with a hand-rolled
parser — far out of scope for a single perf unit and would lose the
discoverability properties of clap-generated `--help`.

The next-largest *avoidable* slice was the ~1.7 % spent in
`tracing_subscriber::registry()` + sharded-slab init. proteus has zero
`tracing::span!` call sites and zero `tracing::info!` call sites; the registry
is overhead at default verbosity. That's what this unit targeted.

## What changed

`src/logging.rs::init` learned a fast path:

```rust
if verbose == 0 && quiet == 0 && rust_log.is_none() && !on_journal {
    return;
}
```

When the user runs proteus interactively without `-v`, without `-q`, without
`RUST_LOG=…`, and not under systemd (no `JOURNAL_STREAM`), `init` returns
without touching the global tracing dispatcher. Every `tracing::*!` macro
becomes a no-op for the rest of the process — including the 16 `tracing::warn!`
sites scattered across `apply`/`revert`/`backend`/`enterprise-wifi` paths.
(Recount with `grep -rn 'tracing::warn!' src/ | wc -l`. Issue #206-G updated
the count after Milestone 1 + 2 added a few more sites.)

This is a deliberate behavior change: warnings emitted via `tracing::warn!`
that previously printed to stderr at default verbosity now don't. They're
recoverable with `proteus -v <cmd>` (DEBUG-and-above) or `RUST_LOG=warn …`.
Errors that the CLI considers user-visible are surfaced via `anyhow` + `eprintln`
and are unaffected. The trade reflects two facts:

1. proteus has zero `info!` and zero `span!` sites, so default-INFO behavior
   never produced any output anyway except for the warn sites.
2. The warn sites are diagnostic hints (e.g. "skip /sys/class/net: …") rather
   than blocking errors. Real failures already propagate via `anyhow` to the
   exit code and stderr.

The wiki page `cli.md` Logging section was updated to document the new
default. No CLI flag, exit code, or JSON shape changed.

## Optimized measurements (same commit + this patch)

Instruction counts, `JOURNAL_STREAM` unset:

| command                | baseline   | optimized  | Δ          | Δ %     |
| ---------------------- | ---------- | ---------- | ---------- | ------- |
| `--help`               | 2,876,662  | 2,876,765  | +103       | +0.00 % |
| `--version`            | 2,876,662  | 2,876,765  | +103       | +0.00 % |
| `show-defaults --json` | 2,741,808  | 2,695,604  | **−46,204**| **−1.69 %** |

`--help` / `--version` are unaffected because clap exits before `cli::run`
reaches `logging::init` — neither code path runs the lazy-init guard. The
+103 instruction delta is from a slightly different binary layout (the two
binaries also differ by 528 bytes; same change).

Instruction counts, `JOURNAL_STREAM` **set** (systemd-launched scenario):

| command                | baseline   | optimized  | Δ        |
| ---------------------- | ---------- | ---------- | -------- |
| `show-defaults --json` | 2,743,822  | 2,743,921  | +99      |

When systemd sets `JOURNAL_STREAM`, the fast path is skipped and we install
the journald subscriber as before. The optimization is correctly inert in
that scenario — exactly the safety property we want for timer-driven runs.

Steady-state RSS — within sampling noise (±8 kB median on 50-sample runs);
not a meaningful change.

Wall-clock — within ±1 % noise band on 500-paired-run interleaved tests.
46 k instructions at ~3 GHz with ~1 IPC is ~15 µs of CPU work, which is lost
in the ~1 ms of ambient process-startup variance.

Binary size: 3,681,584 → 3,681,056 bytes (−528 bytes). Incidental.

## What was *not* changed and why

- **The embedded wiki blob (`include_dir!`) and `WIKI_LINES` table.** The blob
  costs ~zero RSS at runtime when not accessed (kernel pages it in lazily) and
  keeps the "single binary, no external files" property the project depends on.
  Removing it is much bigger work than this unit and the user has explicitly
  said disk size matters less than runtime cost.
- **`Config::default_or_loaded`.** Read-only commands like `show-defaults`
  call `Config::default()` (in-memory), not `default_or_loaded`. The TOML
  parser is only invoked when a command actually needs the user's config;
  the cheap commands already short-circuit.
- **The tokio runtime.** Already deferred — only DBus-touching commands build
  it (`tokio::runtime::Builder::new_current_thread()` is called per-command,
  not in `cli::run`).
- **Replacing `tracing_subscriber::fmt::Subscriber` with a hand-rolled writer.**
  An earlier attempt (now reverted) tried to skip the registry-based
  composition and use `fmt::Subscriber::builder().finish()` directly. It
  turns out `fmt::SubscriberBuilder::finish()` calls
  `with_subscriber(Registry::default())` internally — there is no fmt path
  that avoids the registry+slab. The only way to avoid the slab is to skip
  `init` entirely, which is what the fast path now does.
- **The `release` profile flags.** Already aggressive: `opt-level = "z"`,
  `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`.

## Reproducing these numbers

From the repo root:

```sh
cargo build --release
# Capture the baseline before changing anything.
cp target/release/proteus /tmp/proteus-base

# Apply the patch, then rebuild before copying the optimized binary out.
# Issue #206-F: the rebuild step was previously implicit; without it the
# second `cp` shipped the same binary as the baseline and falsely reported
# zero delta.
cargo build --release
cp target/release/proteus /tmp/proteus-opt

# Instruction counts (deterministic).
env -u JOURNAL_STREAM valgrind --tool=callgrind --callgrind-out-file=/tmp/cg-base.out /tmp/proteus-base show-defaults --json
env -u JOURNAL_STREAM valgrind --tool=callgrind --callgrind-out-file=/tmp/cg-opt.out  /tmp/proteus-opt  show-defaults --json
grep "I   refs" /tmp/cg-base.out /tmp/cg-opt.out  # falls out of valgrind's stderr summary

# Wall-clock (interleaved, 200+ runs each).
# See `tests/perf/perf_compare.sh` if/when this is committed; today it's a
# scratch script described inline in the PR body.

# RSS (steady-state, 50 samples).
for i in $(seq 1 50); do
  env -u JOURNAL_STREAM /usr/bin/time -v /tmp/proteus-base show-defaults --json 2>&1 >/dev/null \
    | awk '/Maximum resident/ {print $NF}'
done | sort -n | awk 'BEGIN{c=0} {a[c++]=$1} END{print "median:", a[int(c/2)]}'
```

Two non-obvious requirements:

1. **`env -u JOURNAL_STREAM`.** Otherwise the systemd-detection path swallows
   the optimization and you'll see no delta.
2. **Same path depth.** Long argv0 paths slow clap's argv parsing on some
   builds; copy both binaries into `/tmp/` before comparing.

## Honest summary

The optimization is real but small (~1.7 % instruction count on the only
sub-1.5 ms command that exercises `cli::run`). On a single-shot interactive
invocation, it's lost in the noise. On automation that runs `proteus` in
tight loops, it adds up to a few percent. The bigger win is structural:
*proteus no longer pays for tracing infrastructure that has no listeners.*

For RSS specifically, the kernel-level cost was always tiny (~kB difference
in a ~970 kB process) because sharded-slab's allocations weren't in the
working set across the cold-start path long enough to matter. The instruction
count is the right metric here, and the savings are honest.
