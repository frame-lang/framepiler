#!/usr/bin/env python3
"""
Validate runnable Frame examples in docs/*.md end-to-end.

For every fenced code block in docs/ that is:
  - a Frame source (contains `@@target python_3`)
  - runnable (contains `if __name__`)
extract it, compile with framec, run with python3, and assert zero exit.

Exits 0 if all pass; 1 if any fails. Prints per-sample status with the
source file and 1-based line range of the opening fence line so failures
point straight at the offending block.

Env:
  FRAMEC    path to framec binary (default: ./target/debug/framec, then
            ./target/release/framec, then `framec` from PATH)
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DOCS_DIR = REPO_ROOT / "docs"


def find_framec() -> str:
    override = os.environ.get("FRAMEC")
    if override:
        return override
    for candidate in (
        REPO_ROOT / "target" / "debug" / "framec",
        REPO_ROOT / "target" / "release" / "framec",
    ):
        if candidate.exists() and os.access(candidate, os.X_OK):
            return str(candidate)
    found = shutil.which("framec")
    if found:
        return found
    sys.stderr.write(
        "error: framec not found. Build with `cargo build` or set FRAMEC=...\n"
    )
    sys.exit(2)


@dataclass
class Sample:
    doc: Path           # markdown file the block came from
    line: int           # 1-based line number of the opening ``` fence
    body: str           # raw code block contents (no fence lines)


def extract_samples(md_path: Path) -> list[Sample]:
    """Yield every fenced code block, regardless of fence language tag."""
    samples: list[Sample] = []
    lines = md_path.read_text().splitlines()
    i = 0
    in_block = False
    block_start = 0
    block_lines: list[str] = []
    while i < len(lines):
        line = lines[i]
        stripped = line.lstrip()
        # Only treat an unindented fence as a real code-block boundary;
        # this avoids matching ``` embedded inside other blocks.
        if stripped.startswith("```") and not line.startswith(" "):
            if in_block:
                samples.append(
                    Sample(
                        doc=md_path,
                        line=block_start,
                        body="\n".join(block_lines),
                    )
                )
                in_block = False
                block_lines = []
            else:
                in_block = True
                block_start = i + 1  # 1-based line of opening fence
        elif in_block:
            block_lines.append(line)
        i += 1
    return samples


def is_runnable_frame(body: str) -> bool:
    return "@@target python_3" in body and "if __name__" in body


def run_sample(framec: str, sample: Sample, workdir: Path) -> tuple[bool, str]:
    """Compile + run one sample. Returns (passed, captured_output)."""
    src = workdir / "sample.fpy"
    out_dir = workdir / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    src.write_text(sample.body)

    # 1) Compile
    compile_proc = subprocess.run(
        [framec, "compile", "-l", "python_3", "-o", str(out_dir), str(src)],
        capture_output=True,
        text=True,
    )
    if compile_proc.returncode != 0:
        return False, (
            "COMPILE FAILED (exit "
            f"{compile_proc.returncode})\n"
            f"stdout:\n{compile_proc.stdout}\n"
            f"stderr:\n{compile_proc.stderr}"
        )

    # framec emits sample.py (same stem as input)
    generated = out_dir / "sample.py"
    if not generated.exists():
        return False, (
            f"COMPILE OK but output file missing: {generated}\n"
            f"dir contents: {[p.name for p in out_dir.iterdir()]}"
        )

    # 2) Run (with timeout to handle non-terminating samples)
    try:
        run_proc = subprocess.run(
            [sys.executable, str(generated)],
            capture_output=True,
            text=True,
            timeout=10,
        )
    except subprocess.TimeoutExpired:
        return False, "RUN FAILED (timeout after 10s — sample may loop)"
    if run_proc.returncode != 0:
        return False, (
            f"RUN FAILED (exit {run_proc.returncode})\n"
            f"stdout:\n{run_proc.stdout}\n"
            f"stderr:\n{run_proc.stderr}"
        )
    return True, run_proc.stdout


def main() -> int:
    framec = find_framec()
    md_files = sorted(DOCS_DIR.glob("*.md"))
    # Also cover the top-level README so the quickstart snippet stays honest.
    readme = REPO_ROOT / "README.md"
    if readme.exists():
        md_files.append(readme)
    if not md_files:
        sys.stderr.write(f"error: no *.md files under {DOCS_DIR}\n")
        return 2

    all_samples: list[Sample] = []
    for md in md_files:
        for s in extract_samples(md):
            if is_runnable_frame(s.body):
                all_samples.append(s)

    if not all_samples:
        print("no runnable Frame samples found")
        return 0

    print(f"Validating {len(all_samples)} runnable sample(s) via {framec}")
    print("-" * 60)

    failures: list[tuple[Sample, str]] = []
    with tempfile.TemporaryDirectory(prefix="framepiler_docs_") as tmpdir:
        tmp = Path(tmpdir)
        for idx, sample in enumerate(all_samples, start=1):
            workdir = tmp / f"s{idx:03d}"
            workdir.mkdir()
            rel = sample.doc.relative_to(REPO_ROOT)
            label = f"{rel}:{sample.line}"
            passed, detail = run_sample(framec, sample, workdir)
            if passed:
                print(f"  ok   {idx:3d}  {label}")
            else:
                print(f"  FAIL {idx:3d}  {label}")
                failures.append((sample, detail))

    print("-" * 60)
    if failures:
        print(f"{len(failures)} failure(s):\n")
        for sample, detail in failures:
            rel = sample.doc.relative_to(REPO_ROOT)
            print(f"=== {rel}:{sample.line} ===")
            print(detail)
            print()
        return 1

    print(f"all {len(all_samples)} samples passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
