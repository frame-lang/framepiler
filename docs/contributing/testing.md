# Testing the Framepiler

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

## Two Levels of Testing

### 1. Unit Tests (in this repo)

```bash
cargo test
```

244 unit tests covering the parser, validator, and codegen internals. Run these before every commit.

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

#### Running Integration Tests

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

The Makefile automatically:
- Cross-compiles framec from your local framepiler source for Linux (Docker containers need a Linux binary)
- Builds language-specific Docker containers (cached after first build)
- Runs tests and reports results

By default it looks for the framepiler source at `../../framepiler` relative to the `docker/` directory. Override with:

```bash
make test FRAMEPILER_SRC=/path/to/your/framepiler
```

#### What the Integration Tests Validate

Each test is a Frame source file (e.g., `.fpy` for Python, `.frs` for Rust) containing:
1. A `@@system` block with the state machine
2. Native epilog code that instantiates the system, calls methods, and checks results

The test runner **transpiles**, **compiles**, and **executes** every test. A test passes only if all three steps succeed and the output indicates PASS.

#### Current Test Counts

| Language | Tests | Status |
|---|---|---|
| Python | 161 | Core |
| TypeScript | 143 | Core |
| JavaScript | 137 | Core |
| Rust | 147 | Core |
| C | 147 | Core |
| C++ | 137 | Core |
| C# | 133 | Core |
| Java | 133 | Core |
| Go | 133 | Core |
| PHP | 133 | Experimental |
| Kotlin | 133 | Experimental |
| Swift | 127 | Experimental |
| Ruby | 136 | Experimental |
| Erlang | 132 | Experimental |
| Lua | 136 | Experimental |
| Dart | 132 | Experimental |
| GDScript | 136 | Experimental |

#### When to Run Integration Tests

- **Before any PR**: Run at least the language(s) affected by your change
- **Backend changes**: Run the specific language — `make test-python` if you changed `backends/python.rs`
- **Core pipeline changes** (parser, validator, codegen framework): Run all — `make test`
- **New tests**: Add test files to `framepiler_test_env/tests/common/positive/<category>/`

#### Interpreting Failures

If a test fails, debug inside the container:

```bash
make shell-python
# Inside the container:
framec compile -l python_3 -o /tmp/out /tests/common/positive/primary/01_interface_return.fpy
python3 /tmp/out/01_interface_return.py
```

Common failure types:
- **Transpile failed**: framec rejected the input or crashed — check the error message
- **Compile failed**: Generated code has syntax errors — the backend emitted invalid target language
- **Runtime error**: Generated code compiles but behaves incorrectly — logic bug in codegen
