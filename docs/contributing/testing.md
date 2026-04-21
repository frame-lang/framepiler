# Testing the Framepiler

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

## Two Levels of Testing

### 1. Unit Tests (in this repo)

```bash
cargo test
```

370 unit tests covering the parser, validator, scanner FSMs, and codegen
internals. Run these before every commit.

### 2. Integration Tests (separate repo)

The full 17-language integration test suite lives in a separate repository:

```
https://github.com/frame-lang/framepiler_test_env
```

Clone it alongside the framepiler repo:

```
projects/
├── framepiler/              # this repo
└── framepiler_test_env/     # integration tests
```

## Running Integration Tests

```bash
cd framepiler_test_env/docker

# First time: build containers (~5 min one-time setup)
make build

# Run all 17 languages against your local framepiler
make test

# Run a single language
make test-python

# See all options
make help
```

The Makefile:

- Cross-compiles framec from your local framepiler source for Linux
  (Docker containers need a Linux binary).
- Builds language-specific Docker images, each containing a pre-built
  TestRunner binary (cached after first build; see *Batched Harness*
  below).
- Runs the chosen language(s)' batched harness and emits TAP.

By default it looks for the framepiler source at `../../framepiler`
relative to the `docker/` directory. Override with:

```bash
make test FRAMEPILER_SRC=/path/to/your/framepiler
```

## Batched Harness

The test harness is **batched** — one compiler invocation + one runtime
process per language, rather than one of each per test. Each language's
flow lives in `docker/runners/<lang>_batch.sh` + `docker/runners/TestRunner.<ext>`
and follows the same three phases:

1. **Transpile** — run `framec` once per `.f*` test file. Output is
   cached by `framec_cached.sh` (keyed on framec binary hash + source
   content hash) so re-runs skip unchanged tests.
2. **Compile** — for compiled languages, one batch invocation
   (`javac`, `kotlinc`, `dotnet build`, `cargo build`, parallel `g++`
   etc.) produces all test binaries / class files.
3. **Dispatch** — `TestRunner.<lang>` iterates a manifest, invokes each
   test's entry point (via reflection for JVM/.NET/Python/Ruby/PHP/Lua;
   `dart <dill>` for Dart; direct exec for native-compiled langs;
   dynamic `import()` for JS/TS), captures stdout/stderr, and emits TAP.

Per-test isolation:
- Each test lives in a unique namespace/package/subdirectory
  (`frametest.t<N>_<name>`) so same-basename tests across categories
  don't collide.
- stdout/stderr is captured per test; crashes don't take out the
  harness.
- Every dispatcher enforces a 30s per-test timeout (overridable via
  `<LANG>_TEST_TIMEOUT` env).
- A trailing integrity check asserts `pass+fail+skip == 1..N` plan;
  silent dispatcher crashes surface as `__harness_integrity__`
  failures.

On compile failure (JVM/.NET/Rust), the batch script scrapes the
error output for offending source files, marks them as `COMPILE_FAIL`,
removes them, and retries once — so one buggy test doesn't poison the
whole batch.

## Current Test Counts

Pass counts from the latest matrix run (2026-04-20, after the async
+ C double + C++23 coroutines + Erlang @@:self work):

| Language   | Passed | Skipped |
|------------|-------:|--------:|
| Python     |    225 |       0 |
| TypeScript |    206 |       0 |
| Rust       |    201 |       0 |
| JavaScript |    200 |       0 |
| Dart       |    199 |       0 |
| GDScript   |    199 |       0 |
| Ruby       |    198 |       1 |
| Lua        |    198 |       1 |
| C#         |    196 |       0 |
| Kotlin     |    196 |       0 |
| Go         |    195 |       1 |
| PHP        |    192 |       4 |
| Swift      |    190 |       0 |
| C          |    208 |       2 |
| C++        |    200 |       0 |
| Java       |    187 |       9 |
| Erlang     |    187 |      11 |
| **TOTAL**  | **3377** | **29** |

All 17 languages are at **zero failures**. Ten languages (Python,
TypeScript, JavaScript, Rust, C++, C#, Kotlin, Swift, Dart, GDScript)
are at **zero skips**. All 29 remaining skips are legitimate
language-incompat — marked with inline comments on each `@@skip`
directive (e.g. "Ruby does not have native async/await", "Erlang
requires one module per file; multi-system tested via other targets").

## When to Run Integration Tests

- **Before any PR**: Run at least the language(s) affected by your change.
- **Backend changes**: Run the specific language — `make test-python` if
  you changed `backends/python.rs`.
- **Core pipeline changes** (scanner, parser, validator, codegen
  framework): Run all — `make test`.
- **New tests**: Add test files to
  `framepiler_test_env/tests/common/positive/<category>/`.

## Interpreting Failures

TAP output looks like:

```
TAP version 14
1..196
ok 1 - 01_traffic_light
ok 2 - 02_toggle_switch
not ok 3 - foo_bar # runtime error (exit 1)
  # actual error output, first 5 lines
# kotlin: 195 passed, 1 failed, 0 skipped
```

Failure reasons the harness emits:
- `# transpile failed` — framec rejected the input.
- `# <tool> failed` (e.g. `# javac failed`, `# kotlinc failed`) —
  generated code didn't compile; framec emitted invalid target code.
- `# runtime error (exit N)` — test binary compiled but exited non-zero.
- `# TIMEOUT` — test exceeded the per-language timeout.
- `# unrecognized output` — test produced neither an `ok` nor a
  `not ok` / `PASS` / `FAIL` marker and exited clean; harness can't
  classify it.
- `# __harness_integrity__ …` — the harness itself detected that fewer
  TAP lines were emitted than declared (silent dispatcher exit).

To reproduce a single failure interactively:

```bash
make shell-python
# Inside the container:
framec compile -l python_3 -o /tmp/out /tests/common/positive/primary/01_interface_return.fpy
python3 /tmp/out/01_interface_return.py
```

## Harness Caveats

- **Static-state leakage.** JVM/.NET/Python/Ruby/PHP/Lua dispatchers
  run tests in a shared interpreter. Frame-generated code is
  instance-based, so leakage is rare, but a test that defines an
  `object` (Kotlin) or a module-global (Python) and mutates it could
  affect subsequent tests. If you hit this, the fast path is to fork
  a process per affected test in the batch script.
- **stdout capture.** Shell-dispatch languages (cpp, rust, swift, go,
  dart, erlang, gdscript) capture per-test output via file redirect
  (`> out.log 2>&1`), preserving NUL bytes and multibyte boundaries.
  In-process dispatchers use their language's native capture APIs
  (`StringIO` / `PrintStream`/etc.).
- **`process.exit()` in tests.** The TypeScript / JavaScript TestRunner
  intercepts `process.exit()` so a failing test can't take down the
  node harness. If you see tests silently disappearing mid-run, check
  that intercept is still wired up.
