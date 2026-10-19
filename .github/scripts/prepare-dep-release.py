#!/usr/bin/env python3
"""Work out the next version for a dependency release and finalize CHANGELOG.md.

Called by .github/workflows/dep-release.yml after Dependabot pull requests have
been merged into master. Reports `release` and `version` through GITHUB_OUTPUT
and leaves the tree untouched when there is nothing to release.

Two cases:

* The newest CHANGELOG section is already tagged, so the merged bumps are the
  only unreleased change: a new patch section is opened for them.
* The newest section has no tag yet (a feature release is waiting on master):
  the bumps are folded into that section and it is released under its own
  version, because a build of master ships those changes either way.
"""

import json
import os
import re
import subprocess
import sys
from datetime import date

REPO_URL = "https://github.com/mhoogenbosch/TEAMS2HA"
CHANGELOG = "CHANGELOG.md"

SECTION = re.compile(r"^## \[(v\d+\.\d+\.\d+)\]")
SECTION_DATE = re.compile(r"^(## \[v\d+\.\d+\.\d+\]\s+—\s+)\d{4}-\d{2}-\d{2}(.*)$")
SEMVER_TAG = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")
LINK_LINE = re.compile(r"^\[v\d+\.\d+\.\d+\]:")
# "chore(deps): bump x from 1 to 2", "chore(deps-dev): ...", "build(deps): ...",
# plus Dependabot's own unprefixed "Bump x from 1 to 2".
DEP_PREFIX = re.compile(r"^(?:chore|build|ci)\((?:deps|deps-dev|actions)\):\s*", re.I)
BUMP_SUBJECT = re.compile(r"^bump\s", re.I)


def git(*args: str) -> str:
    return subprocess.run(
        ("git",) + args, capture_output=True, text=True, check=True
    ).stdout.strip()


def emit(**values: str) -> None:
    for key, value in values.items():
        print(f"{key}={value}")
    path = os.environ.get("GITHUB_OUTPUT")
    if path:
        with open(path, "a", encoding="utf-8") as handle:
            for key, value in values.items():
                handle.write(f"{key}={value}\n")


def latest_tag() -> str | None:
    tags = [t for t in git("tag", "--list", "v*").splitlines() if SEMVER_TAG.match(t)]
    if not tags:
        return None
    return max(tags, key=lambda t: tuple(int(p) for p in SEMVER_TAG.match(t).groups()))


def next_patch(tag: str) -> str:
    major, minor, patch = (int(p) for p in SEMVER_TAG.match(tag).groups())
    return f"v{major}.{minor}.{patch + 1}"


def commit_subjects(since: str) -> list[str]:
    return [s.strip() for s in git("log", f"{since}..HEAD", "--format=%s").splitlines()]


def dependency_bullets(subjects: list[str]) -> tuple[list[str], list[str]]:
    """Split commit subjects into dependency bullets (newest first) and the rest."""
    bullets: list[str] = []
    others: list[str] = []
    for subject in subjects:
        stripped = DEP_PREFIX.sub("", subject)
        if stripped == subject and not BUMP_SUBJECT.match(subject):
            others.append(subject)
            continue
        text = stripped[:1].upper() + stripped[1:]
        if text not in bullets:
            bullets.append(text)
    return bullets, others


def section_bounds(lines: list[str], start: int) -> int:
    """Index of the line after the section that starts at `start`."""
    for index in range(start + 1, len(lines)):
        if SECTION.match(lines[index]):
            return index
    return len(lines)


def insert_bullets(lines: list[str], start: int, end: int, bullets: list[str]) -> None:
    """Add `bullets` to the section's ### Dependencies block, creating it if needed."""
    heading = None
    for index in range(start, end):
        if lines[index].strip().lower() == "### dependencies":
            heading = index
            break

    if heading is None:
        at = end
        while at > start and not lines[at - 1].strip():
            at -= 1  # insert above the blank line that separates the sections
        lines[at:at] = ["### Dependencies", *(f"- {b}" for b in bullets)]
        return

    at = heading + 1
    existing = []
    while at < end and (lines[at].startswith("- ") or lines[at].startswith("  ")):
        existing.append(lines[at].removeprefix("- ").strip())
        at += 1
    fresh = [b for b in bullets if b not in existing]
    lines[at:at] = [f"- {b}" for b in fresh]


def add_link(lines: list[str], version: str) -> None:
    link = f"[{version}]: {REPO_URL}/releases/tag/{version}"
    if any(line.startswith(f"[{version}]:") for line in lines):
        return
    for index, line in enumerate(lines):
        if LINK_LINE.match(line):
            lines.insert(index, link)
            return
    lines.extend(["", link])


def tauri_mismatches() -> list[str]:
    """Tauri pairs whose npm package and Rust crate differ in major.minor.

    `tauri build` refuses such a pair ("Found version mismatched Tauri packages").
    The npm and cargo groups arrive as separate Dependabot pull requests, so after
    one of them merges, master can briefly hold e.g. @tauri-apps/plugin-updater
    2.14 next to tauri-plugin-updater 2.13. Releasing then tags a version whose
    build is certain to fail: v1.5.21 on 2026-10-19. Waiting is enough, because
    the other group's merge dispatches this workflow again.
    """
    try:
        lock = json.load(open("tauri/package-lock.json", encoding="utf-8"))
        cargo = open("tauri/src-tauri/Cargo.lock", encoding="utf-8").read()
    except OSError as err:
        print(f"::warning::Tauri version check skipped: {err}")
        return []
    npm = {
        path.removeprefix("node_modules/@tauri-apps/"): meta.get("version", "")
        for path, meta in lock.get("packages", {}).items()
        if path.startswith("node_modules/@tauri-apps/") and path.count("node_modules/") == 1
    }
    crates = dict(re.findall(r'\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"', cargo))

    def minor(version: str) -> str:
        return ".".join(version.split(".")[:2])

    pairs = {"api": "tauri"} | {
        name: f"tauri-{name}" for name in npm if name.startswith("plugin-")
    }
    return [
        f"{crate} ({crates[crate]}) : @tauri-apps/{name} ({npm[name]})"
        for name, crate in sorted(pairs.items())
        if name in npm and crate in crates and minor(npm[name]) != minor(crates[crate])
    ]


def main() -> int:
    tag = latest_tag()
    if tag is None:
        print("::error::No vX.Y.Z tag found — cannot derive the next version.")
        emit(release="false")
        return 1

    bullets, others = dependency_bullets(commit_subjects(tag))
    if not bullets:
        print(f"No dependency commits since {tag} — nothing to release.")
        emit(release="false")
        return 0

    mismatched = tauri_mismatches()
    if mismatched:
        for pair in mismatched:
            print(f"::notice::Waiting for the matching bump: {pair}")
        print("Not releasing yet: the npm and Rust sides of Tauri differ in major.minor, so the "
              "build would fail. The other ecosystem's Dependabot merge will start this again.")
        emit(release="false")
        return 0

    lines = open(CHANGELOG, encoding="utf-8").read().split("\n")
    top = next((i for i, line in enumerate(lines) if SECTION.match(line)), None)
    if top is None:
        print(f"::error::No '## [vX.Y.Z]' section found in {CHANGELOG}.")
        emit(release="false")
        return 1

    top_version = SECTION.match(lines[top]).group(1)
    today = date.today().isoformat()

    if top_version == tag:
        # Everything on master ships in this build, so flag anything that is not
        # a bump: it would go out under a "dependency updates" heading.
        for subject in others:
            print(f"::warning::Undocumented non-dependency commit ships along: {subject}")
        version = next_patch(tag)
        lines[top:top] = [
            f"## [{version}] — {today} (dependency updates)",
            "### Dependencies",
            *(f"- {b}" for b in bullets),
            "",
        ]
    else:
        # Unreleased section on master: release it, with the bumps folded in.
        # But NEVER re-release a version that is already tagged: that happened on
        # 2026-09-07, when a missing CHANGELOG section for the newest tag made the
        # top section (v1.5.9, long since released) look unreleased — the tag was
        # then force-moved and the published assets overwritten. A tagged top
        # section means the CHANGELOG is behind, and the fix is a missing section,
        # not a re-tag.
        if git("tag", "--list", top_version):
            print(
                f"::error::{CHANGELOG}'s newest section {top_version} is already "
                f"tagged while the newest tag is {tag} — a CHANGELOG section for "
                f"{tag} is missing. Refusing to re-release {top_version}; add the "
                "missing section(s) first."
            )
            emit(release="false")
            return 1
        version = top_version
        dated = SECTION_DATE.match(lines[top])
        if dated:
            lines[top] = f"{dated.group(1)}{today}{dated.group(2)}"
        insert_bullets(lines, top, section_bounds(lines, top), bullets)

    add_link(lines, version)
    open(CHANGELOG, "w", encoding="utf-8").write("\n".join(lines))

    print(f"Releasing {version} with {len(bullets)} dependency change(s) since {tag}.")
    emit(release="true", version=version)
    return 0


if __name__ == "__main__":
    sys.exit(main())
