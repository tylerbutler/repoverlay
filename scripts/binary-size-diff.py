#!/usr/bin/env python3
"""Calculate binary size difference and output for GitHub Actions."""

import argparse
import sys


def human_size(size_bytes: int) -> str:
    """Convert bytes to human-readable format."""
    for unit in ["B", "KB", "MB", "GB"]:
        if abs(size_bytes) < 1024:
            return f"{size_bytes:.1f}{unit}" if unit != "B" else f"{size_bytes}{unit}"
        size_bytes /= 1024
    return f"{size_bytes:.1f}TB"


def main() -> int:
    parser = argparse.ArgumentParser(description="Calculate binary size difference")
    parser.add_argument("pr_size", type=int, help="PR binary size in bytes")
    parser.add_argument("main_size", type=int, help="Main branch binary size in bytes")
    parser.add_argument(
        "--threshold-percent",
        type=float,
        default=5.0,
        help="Percentage threshold for significant change",
    )
    parser.add_argument(
        "--threshold-bytes",
        type=int,
        default=51200,
        help="Byte threshold for significant change",
    )
    parser.add_argument(
        "--github-output",
        type=str,
        help="Path to GITHUB_OUTPUT file",
    )
    args = parser.parse_args()

    pr_size = args.pr_size
    main_size = args.main_size
    diff = pr_size - main_size

    pr_human = human_size(pr_size)
    main_human = human_size(main_size)
    diff_human = human_size(abs(diff))
    if diff < 0:
        diff_human = f"-{diff_human}"
    elif diff > 0:
        diff_human = f"+{diff_human}"

    percent = (diff / main_size * 100) if main_size > 0 else 0
    percent_str = f"+{percent:.1f}%" if diff > 0 else f"{percent:.1f}%"

    # Determine if change is significant
    abs_diff = abs(diff)
    abs_percent = abs(percent)
    significant = abs_diff > args.threshold_bytes or abs_percent > args.threshold_percent

    # Output results
    output_lines = [
        f"pr_size={pr_human}",
        f"main_size={main_human}",
        f"diff={diff_human}",
        f"diff_bytes={diff}",
        f"percent={percent_str}",
        f"significant={'true' if significant else 'false'}",
    ]

    if args.github_output:
        with open(args.github_output, "a") as f:
            f.write("\n".join(output_lines) + "\n")
    else:
        for line in output_lines:
            print(line)

    return 0


if __name__ == "__main__":
    sys.exit(main())
