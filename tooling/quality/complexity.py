"""Enforce source-level cyclomatic complexity using Mozilla rust-code-analysis."""

import json
import os
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

LIMIT = 15
EXTENSIONS = {
    ".rs",
    ".ts",
    ".tsx",
    ".js",
    ".jsx",
    ".mjs",
    ".cjs",
    ".mts",
    ".cts",
    ".py",
}
ARTIFACT_DIRS = {
    ".git",
    ".venv",
    "node_modules",
    "target",
    "dist",
    ".output",
    ".tanstack",
    "__pycache__",
    ".ruff_cache",
    ".pytest_cache",
    ".superpowers",
    ".tools",
    "test-results",
    "playwright-report",
}
GENERATED_FILES = {"apps/web/src/routeTree.gen.ts"}
ALIASES = {".mjs": ".js", ".cjs": ".js", ".mts": ".ts", ".cts": ".ts"}
ANALYZER = Path(__file__).resolve().parents[2] / ".tools/bin/rust-code-analysis-cli"


@dataclass(frozen=True)
class FunctionMetric:
    name: str
    start_line: int
    cyclomatic_complexity: int


def function_metrics(space: dict) -> list[FunctionMetric]:
    functions = []
    if space["kind"] in {"function", "closure"}:
        # max preserves violations inside nested functions; never gate on averages.
        functions.append(
            FunctionMetric(
                space["name"] or "<anonymous>",
                space["start_line"],
                int(space["metrics"]["cyclomatic"]["max"]),
            )
        )
    for child in space["spaces"]:
        functions.extend(function_metrics(child))
    return functions


def source_files(root: Path) -> list[Path]:
    files = []
    for directory, children, names in os.walk(root):
        children[:] = sorted(set(children) - ARTIFACT_DIRS)
        for name in sorted(names):
            path = Path(directory) / name
            if path.suffix not in EXTENSIONS:
                continue
            if path.relative_to(root).as_posix() not in GENERATED_FILES:
                files.append(path)
    return files


def analyze(filename: str, source: str) -> list[FunctionMetric]:
    path = Path(filename)
    suffix = ALIASES.get(path.suffix, path.suffix)
    with tempfile.TemporaryDirectory(prefix="cubebot-complexity-") as directory:
        normalized = Path(directory) / ("source" + suffix)
        normalized.write_text(source, encoding="utf-8")
        syntax = subprocess.run(
            [str(ANALYZER), "-p", str(normalized), "--find", "ERROR", "-j", "1"],
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        )
        if syntax.stdout.strip():
            raise ValueError(f"Complexity parser could not understand {filename}")
        result = subprocess.run(
            [str(ANALYZER), "-m", "-p", str(normalized), "-O", "json", "-j", "1"],
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        )
    return function_metrics(json.loads(result.stdout))


def check(root: Path) -> int:
    files = source_files(root)
    if not files:
        print(f"No source files found under {root}", file=sys.stderr)
        return 2
    violations = 0
    count = 0
    maximum = 0
    for path in files:
        functions = analyze(str(path), path.read_text(encoding="utf-8"))
        for function in functions:
            count += 1
            complexity = function.cyclomatic_complexity
            maximum = max(maximum, complexity)
            if complexity > LIMIT:
                violations += 1
                print(
                    f"{path.relative_to(root)}:{function.start_line}: "
                    f"{function.name}: {complexity} > {LIMIT}"
                )
    print(
        f"Complexity: {len(files)} files, {count} functions, "
        f"maximum {maximum}, limit {LIMIT}"
    )
    return int(violations > 0)


if __name__ == "__main__":
    root = (
        Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    )
    sys.exit(check(root.resolve()))
