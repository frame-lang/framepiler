# Frame Roadmap

A living document of what's shipping, what's next, and what's deferred. Updated as priorities shift.

**Last updated**: 2026-05-10

---

## Available today

- **Frame language** with `@@system` blocks, hierarchical state machines, push/pop modal stack, system composition, persist contract (RFC-0015), annotations (RFC-0013).
- **17 target languages**: Python, TypeScript, JavaScript, Rust, C, C++, Java, C#, Go, PHP, Kotlin, Swift, Ruby, Erlang, Lua, Dart, GDScript.
- **Graphviz output** for state-diagram visualization.
- **`cargo install framec`** — Rust developers can install the transpiler from crates.io. Source-distribution: users compile locally.
- **Prebuilt binaries on GitHub Releases** — first 6-platform release shipping as `v4.1.2-rc1` (May 2026). Covers macOS x86_64/aarch64, Linux x86_64/aarch64, Windows x86_64/aarch64. Each platform's tarball/zip is checksummed (SHA-256) and attested for build provenance via SLSA.
- **111 cookbook recipes** demonstrating Frame patterns from basic state machines through enterprise integration patterns.
- **Per-language guides** for every backend with target-specific notes, gotchas, and idiom catalogs.
- **17-language differential test harness** (`framec-test-env`) with 396 unit tests + 321 common positive fixtures × 17 backends + 33,000+ fuzz cases across 21 generators covering correctness axes.

---

## Near-term (next 1–3 months)

### Distribution — making framec installable from any package manager

The 6-platform prebuilt binaries are shipping as of `v4.1.2-rc1`. The remaining work is wrapping those binaries with the package managers most developers actually reach for:

- [ ] **Promote `v4.1.2` from release candidate to stable** once rc1 has soaked.
- [ ] **`brew install frame-lang/tap/framec`** — Homebrew tap for macOS and Linux.
- [ ] **`winget install framec`** and **`scoop install framec`** — Windows package managers.
- [ ] **`install.sh`** — Linux convenience installer (curl-piped-to-shell pattern).

### Hardening

- [ ] **Trusted Publishing on crates.io** — replace long-lived publish tokens with OIDC.
- [ ] **Dependabot** for Cargo + GitHub Actions dependency updates.
- [ ] **CodeQL** static analysis on every PR + main.
- [ ] **Branch protection on main** with required CI checks + linear history.

---

## Medium-term (next 3–6 months)

### WASM distribution — same artifact, multiple audiences

framec's library entry point (`lib.rs`) is structured to compile to WASM. We haven't yet shipped a published WASM artifact, but the plan is one build, four audiences:

- [ ] **`npm install -g @frame-lang/framec`** — JavaScript/TypeScript audience. Single npm package wrapping the WASM build with a Node CLI shim.
- [ ] **`pip install framec`** — Python audience. Same WASM, wrapped via `wasmtime-py`.
- [ ] **Browser playground** at `frame-lang.org/playground` — type Frame source in the left pane, see generated code in the right pane, switch target languages from a dropdown to demonstrate portability. Loads the same WASM from CDN.
- [ ] **VS Code extension** migration to consume the published `@frame-lang/framec` package instead of bundling its own WASM build.

### Code signing (when prebuilt-binary path becomes the primary install)

- [ ] **macOS notarization** — Apple Developer Program enrollment + Developer ID Application cert.
- [ ] **Windows code signing certificate** — OV or EV cert for SmartScreen-clean Windows binaries.

Both deferred until prebuilt binaries see real adoption; the WASM track requires neither.

---

## Long-term (when bandwidth allows)

- [ ] **RFC-0016 — Selective domain persist** (`@@[persist_fields([...])]`). Designed; deferred from 4.0/4.1.
- [ ] **More cookbook recipes** — extend the 111 with patterns surfaced by community use.
- [ ] **Community package channels** — accept PRs for AUR, Nixpkgs, Chocolatey, Debian/Ubuntu distro packaging if contributors offer them.
- [ ] **GitHub Discussions** for community Q&A separate from issues.
- [ ] **Issue templates polish** — backend request, RFC, etc.

---

## Won't do (explicitly deferred)

These come up frequently enough to be worth saying "no" to:

- **DCO bot / CLA assistant**: solo BDFL project; not warranted.
- **Stale-issue auto-close bot**: issue volume doesn't justify the friction.
- **Multi-Rust-version CI matrix**: targeting one stable Rust version is fine.
- **Per-language native binaries via npm `optionalDependencies`** (the esbuild / swc / biome pattern): WASM is fast enough for Frame's one-shot codegen usage profile. Reconsider only if real perf complaints surface.
- **Code signing for the WASM track**: provenance attestations (free, OIDC-based) are the equivalent for sandboxed runtimes.

---

## How priorities shift

This roadmap is non-binding. Priority moves based on:

- **User signal**: an issue with multiple reactions, recurring questions in Discussions, or a real production blocker can promote a long-term item to near-term.
- **Hardening triggers**: security advisories or supply-chain concerns can promote hardening work to immediate.
- **Bandwidth**: solo maintenance means parallel work is limited; some quarters are "stabilize what we have," others are "ship new distribution channels."

If something on this list matters to you, [open an issue](https://github.com/frame-lang/framec/issues) — that's the strongest signal for what should ship next.

---

## License

[Apache 2.0](./LICENSE)
