#!/usr/bin/env python3
"""Verify that generated configs match commit-types.json (single source of truth)."""

import subprocess
import sys
from pathlib import Path


def get_generated_content(script: str) -> str:
    """Run a generator script with --dry-run and capture output."""
    result = subprocess.run(
        ["python3", script, "--dry-run"],
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout


def main():
    root = Path(__file__).parent.parent
    errors = []

    # Check commitlint config
    commitlint_path = root / ".commitlintrc.json"
    if commitlint_path.exists():
        expected = get_generated_content(root / "scripts" / "generate-commitlint-config.py")
        actual = commitlint_path.read_text()

        if expected.strip() != actual.strip():
            errors.append(
                ".commitlintrc.json out of sync. Run: python3 scripts/generate-commitlint-config.py"
            )
    else:
        errors.append(
            ".commitlintrc.json does not exist. Run: python3 scripts/generate-commitlint-config.py"
        )

    # Check cliff config
    cliff_path = root / "cliff.toml"
    if cliff_path.exists():
        expected = get_generated_content(root / "scripts" / "generate-cliff-configs.py")
        actual = cliff_path.read_text()

        if expected.strip() != actual.strip():
            errors.append(
                "cliff.toml out of sync. Run: python3 scripts/generate-cliff-configs.py"
            )
    else:
        errors.append(
            "cliff.toml does not exist. Run: python3 scripts/generate-cliff-configs.py"
        )

    if errors:
        print("Config sync check failed:\n")
        for error in errors:
            print(f"  - {error}\n")
        sys.exit(1)
    else:
        print("All configs are in sync with commit-types.json")
        sys.exit(0)


if __name__ == "__main__":
    main()
