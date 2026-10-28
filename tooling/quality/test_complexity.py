"""Exercise the actual analyzer and command, including inclusive threshold edges."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from complexity import LIMIT, analyze, source_files

CHECKER = Path(__file__).with_name("complexity.py")


def sample(extension: str, branches: int) -> str:
    if extension == ".py":
        body = "".join(
            f"    if x == {n}:\n        return {n}\n" for n in range(branches)
        )
        return f"def example(x):\n{body}    return -1\n"
    body = "".join(f"if (x == {n}) {{ return {n}; }}\n" for n in range(branches))
    if extension == ".rs":
        return f"fn example(x: i32) -> i32 {{ {body} -1 }}"
    return f"function example(x) {{ {body} return -1; }}"


class ComplexityTests(unittest.TestCase):
    def test_inclusive_threshold_for_every_source_language(self):
        for extension in (
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
        ):
            for expected in (15, 16):
                with self.subTest(extension=extension, expected=expected):
                    functions = analyze(
                        "sample" + extension, sample(extension, expected - 1)
                    )
                    self.assertEqual(
                        [item.cyclomatic_complexity for item in functions], [expected]
                    )
                    self.assertEqual(
                        functions[0].cyclomatic_complexity > LIMIT, expected == 16
                    )

    def test_only_new_src_directories_are_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            names = (
                "new-module/src/lib.rs",
                "apps/web/src/new.tsx",
                "tooling/src/check.py",
                "apps/agent/tests/case.rs",
                "tooling/check.py",
                "src.rs",
                "target/src/generated.rs",
                "apps/web/node_modules/library/index.js",
                "apps/web/src/routeTree.gen.ts",
            )
            for name in names:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("", encoding="utf-8")
            self.assertEqual(
                {path.relative_to(root).as_posix() for path in source_files(root)},
                set(names[:3]),
            )

    def test_command_fails_for_one_violation_even_among_simple_functions(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "src/main.rs"
            source.parent.mkdir()
            source.write_text(sample(".rs", 14), encoding="utf-8")
            self.assertEqual(self.run_checker(directory).returncode, 0)
            source.write_text(sample(".rs", 15) + "\nfn simple() {}", encoding="utf-8")
            result = self.run_checker(directory)
            self.assertEqual(result.returncode, 1)
            self.assertIn("16 > 15", result.stdout)

    def test_empty_scan_is_an_error(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertNotEqual(self.run_checker(directory).returncode, 0)

    def test_rust_match_arms_cannot_bypass_limit(self):
        for expected in (15, 16):
            arms = ",".join(f"{n} => {n}" for n in range(expected - 2))
            source = f"fn example(x:i32)->i32 {{ match x {{ {arms}, _ => 0 }} }}"
            functions = analyze("match.rs", source)
            self.assertEqual(functions[0].cyclomatic_complexity, expected)

    def test_parse_errors_fail_closed(self):
        with self.assertRaisesRegex(ValueError, "could not understand"):
            analyze("broken.rs", "fn broken( { this is not Rust;")

    def test_anonymous_arrow_functions_are_counted(self):
        body = "".join(f"if (x === {n}) {{ return {n}; }}" for n in range(15))
        source = f"export const example = (x: number) => {{ {body} return -1; }}"
        functions = analyze("arrow.ts", source)
        self.assertEqual(functions[0].cyclomatic_complexity, 16)

    def test_suppression_comments_cannot_hide_functions(self):
        source = "// #lizard forgives\n" + sample(".rs", 15)
        functions = analyze("suppressed.rs", source)
        self.assertEqual(len(functions), 1)
        self.assertEqual(functions[0].cyclomatic_complexity, 16)

    @staticmethod
    def run_checker(directory: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(CHECKER), directory],
            check=False,
            capture_output=True,
            text=True,
        )


if __name__ == "__main__":
    unittest.main()
