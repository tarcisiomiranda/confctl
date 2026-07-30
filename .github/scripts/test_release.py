"""Tests for the confctl GitHub release publisher."""

from __future__ import annotations

import hashlib
import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).with_name("release.py")
SPEC = importlib.util.spec_from_file_location("confctl_release", SCRIPT_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"could not load {SCRIPT_PATH}")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ValidateTagTests(unittest.TestCase):
    def test_accepts_semver_tags(self) -> None:
        for tag in ("v0.1.0", "v1.2.3", "v2.0.0-rc.1", "v3.4.5+build.7"):
            with self.subTest(tag=tag):
                self.assertEqual(release.validate_tag(tag), tag)

    def test_rejects_invalid_tags(self) -> None:
        for tag in ("1.2.3", "v1.2", "v01.2.3", "latest", "v1.2.3-"):
            with self.subTest(tag=tag):
                with self.assertRaises(ValueError):
                    release.validate_tag(tag)


class YamlNotesTests(unittest.TestCase):
    def test_parse_release_notes_yaml_dialect(self) -> None:
        text = """
# comment
highlights: |
  Partial redact and DB URLs.

features:
  - first feature
  - "second: with colon"

fixes:
  - redact short strings

previous: v0.0.4
"""
        data = release.parse_release_notes_yaml(text)
        self.assertIn("Partial redact", data["highlights"])
        self.assertEqual(data["features"], ["first feature", "second: with colon"])
        self.assertEqual(data["fixes"], ["redact short strings"])
        self.assertEqual(data["previous"], "v0.0.4")

    def test_build_body_from_notes(self) -> None:
        body = release.build_release_body_from_notes(
            {
                "title": "Redact improvements",
                "highlights": "Safer agent context.",
                "features": ["DB URL password mask", "Partial -r N"],
                "fixes": ["Too-short strings fall back to full mask"],
                "changes": ["Agent skills installer"],
            },
            "v0.0.5",
            "v0.0.6",
            "owner/confctl",
        )
        self.assertIn("## Redact improvements", body)
        self.assertIn("Safer agent context.", body)
        self.assertIn("### Features\n- DB URL password mask\n- Partial -r N", body)
        self.assertIn("### Fixes\n- Too-short strings fall back to full mask", body)
        self.assertIn("### Changes\n- Agent skills installer", body)
        self.assertIn(
            "https://github.com/owner/confctl/compare/v0.0.5...v0.0.6",
            body,
        )

    def test_notes_file_preferred_over_commits(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            notes_dir = root / "releases"
            notes_dir.mkdir()
            (notes_dir / "v0.0.6.yaml").write_text(
                "features:\n  - from yaml file\n",
                encoding="utf-8",
            )
            body = release.release_notes("v0.0.6", "owner/confctl", root=root)
            self.assertIsNotNone(body)
            assert body is not None
            self.assertIn("from yaml file", body)
            self.assertIn("### Features", body)

    def test_rejects_mismatched_tag_field(self) -> None:
        with self.assertRaises(ValueError):
            release.build_release_body_from_notes(
                {"tag": "v0.0.4", "features": ["x"]},
                None,
                "v0.0.5",
                "owner/confctl",
            )


class ReleaseBodyTests(unittest.TestCase):
    def test_groups_conventional_commits(self) -> None:
        body = release.build_release_body(
            [
                "feat: add partial redact",
                "feat(cli): optional -r percent",
                "fix: short string mask",
                "docs: update README",
                "plain subject without prefix",
            ],
            "v0.0.5",
            "v0.0.6",
            "owner/confctl",
        )

        self.assertIsNotNone(body)
        assert body is not None
        self.assertIn("## What's new", body)
        self.assertIn("### Features\n- add partial redact\n- optional -r percent", body)
        self.assertIn("### Fixes\n- short string mask", body)
        self.assertIn(
            "### Other changes\n- update README\n- plain subject without prefix",
            body,
        )
        self.assertIn(
            "https://github.com/owner/confctl/compare/v0.0.5...v0.0.6",
            body,
        )

    def test_returns_none_without_commits(self) -> None:
        self.assertIsNone(
            release.build_release_body([], "v0.0.5", "v0.0.6", "owner/confctl")
        )

    def test_first_release_has_no_compare_link(self) -> None:
        body = release.build_release_body(
            ["feat: initial"], None, "v0.0.1", "owner/confctl"
        )
        assert body is not None
        self.assertNotIn("compare", body)

    def test_skips_empty_sections(self) -> None:
        body = release.build_release_body(
            ["fix: only a fix"], "v0.0.5", "v0.0.6", "owner/confctl"
        )
        assert body is not None
        self.assertNotIn("### Features", body)
        self.assertNotIn("### Other changes", body)


class ChangelogCommitsTests(unittest.TestCase):
    def test_reads_range_from_git_history(self) -> None:
        import os
        import subprocess

        with tempfile.TemporaryDirectory() as temporary_directory:
            env = {
                **os.environ,
                "GIT_AUTHOR_NAME": "t",
                "GIT_AUTHOR_EMAIL": "t@example.com",
                "GIT_COMMITTER_NAME": "t",
                "GIT_COMMITTER_EMAIL": "t@example.com",
            }

            def git(*args: str) -> None:
                subprocess.run(
                    ["git", *args],
                    cwd=temporary_directory,
                    env=env,
                    check=True,
                    capture_output=True,
                )

            git("init", "-q")
            git("commit", "--allow-empty", "-q", "-m", "feat: first")
            git("tag", "v0.1.0")
            git("commit", "--allow-empty", "-q", "-m", "feat: second")
            git("commit", "--allow-empty", "-q", "-m", "fix: third")
            git("tag", "v0.2.0")

            cwd = os.getcwd()
            os.chdir(temporary_directory)
            try:
                previous, subjects = release.changelog_commits("v0.2.0")
            finally:
                os.chdir(cwd)

        self.assertEqual(previous, "v0.1.0")
        self.assertEqual(subjects, ["fix: third", "feat: second"])


class AssetValidationTests(unittest.TestCase):
    def test_accepts_complete_release_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            release_dir = Path(temporary_directory)
            checksums = []
            for filename in sorted(release.EXPECTED_BINARIES):
                contents = f"binary:{filename}".encode()
                (release_dir / filename).write_bytes(contents)
                checksums.append(f"{hashlib.sha256(contents).hexdigest()}  {filename}")
            (release_dir / "checksums.txt").write_text(
                "\n".join(checksums) + "\n",
                encoding="utf-8",
            )

            assets = release.collect_and_verify_assets(release_dir)

            self.assertEqual(len(assets), len(release.EXPECTED_BINARIES) + 1)
            self.assertEqual(assets[-1].name, "checksums.txt")

    def test_rejects_checksum_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            release_dir = Path(temporary_directory)
            checksums = []
            for filename in sorted(release.EXPECTED_BINARIES):
                (release_dir / filename).write_bytes(b"binary")
                checksums.append(f"{'0' * 64}  {filename}")
            (release_dir / "checksums.txt").write_text(
                "\n".join(checksums) + "\n",
                encoding="utf-8",
            )

            with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
                release.collect_and_verify_assets(release_dir)

    def test_expected_confctl_binaries(self) -> None:
        self.assertEqual(
            release.EXPECTED_BINARIES,
            {
                "confctl-linux-amd64",
                "confctl-darwin-arm64",
                "confctl-darwin-amd64",
            },
        )


class NextPatchTagTests(unittest.TestCase):
    def test_next_patch_from_latest(self) -> None:
        import os
        import subprocess

        with tempfile.TemporaryDirectory() as temporary_directory:
            env = {
                **os.environ,
                "GIT_AUTHOR_NAME": "t",
                "GIT_AUTHOR_EMAIL": "t@example.com",
                "GIT_COMMITTER_NAME": "t",
                "GIT_COMMITTER_EMAIL": "t@example.com",
            }

            def git(*args: str) -> None:
                subprocess.run(
                    ["git", *args],
                    cwd=temporary_directory,
                    env=env,
                    check=True,
                    capture_output=True,
                )

            git("init", "-q")
            git("commit", "--allow-empty", "-q", "-m", "init")
            git("tag", "v0.0.5")

            cwd = os.getcwd()
            os.chdir(temporary_directory)
            try:
                self.assertEqual(release.next_patch_tag(), "v0.0.6")
            finally:
                os.chdir(cwd)


if __name__ == "__main__":
    unittest.main()
