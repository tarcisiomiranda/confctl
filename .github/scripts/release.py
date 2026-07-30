#!/usr/bin/env python3
"""Validate artifacts and publish a confctl GitHub release.

Prefers hand-written notes from releases/<tag>.yaml; falls back to
conventional-commit subjects since the previous SemVer tag.

Also supports:
  --validate-tag TAG   check SemVer shape without contacting GitHub
  --bump-tag           create and push the next patch tag (for tag.yml)
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen


TAG_PATTERN = re.compile(
    r"^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
EXPECTED_BINARIES = {
    "confctl-linux-amd64",
    "confctl-darwin-arm64",
    "confctl-darwin-amd64",
}


class GitHubAPIError(RuntimeError):
    """An unsuccessful GitHub API response."""

    def __init__(self, status: int, body: str):
        super().__init__(f"GitHub API returned HTTP {status}: {body}")
        self.status = status


class GitHubAPI:
    """Minimal GitHub Releases API client using only the standard library."""

    def __init__(self, token: str, api_url: str):
        self.token = token
        self.api_url = api_url.rstrip("/")

    def _headers(self, content_type: str = "application/json") -> dict[str, str]:
        return {
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {self.token}",
            "Content-Type": content_type,
            "User-Agent": "confctl-release-script",
            "X-GitHub-Api-Version": "2022-11-28",
        }

    def request_json(
        self,
        method: str,
        url: str,
        payload: dict[str, Any] | None = None,
    ) -> dict[str, Any] | list[dict[str, Any]] | None:
        data = json.dumps(payload).encode() if payload is not None else None
        request = Request(
            url,
            data=data,
            headers=self._headers(),
            method=method,
        )
        try:
            with urlopen(request, timeout=30) as response:
                body = response.read()
        except HTTPError as error:
            body = error.read().decode(errors="replace")
            raise GitHubAPIError(error.code, body) from error
        except URLError as error:
            raise RuntimeError(f"GitHub API request failed: {error.reason}") from error

        if not body:
            return None
        return json.loads(body)

    def upload_asset(self, upload_url: str, asset_path: Path) -> dict[str, Any]:
        base_url = upload_url.split("{", 1)[0]
        url = f"{base_url}?{urlencode({'name': asset_path.name})}"
        request = Request(
            url,
            data=asset_path.read_bytes(),
            headers=self._headers("application/octet-stream"),
            method="POST",
        )
        try:
            with urlopen(request, timeout=120) as response:
                return json.loads(response.read())
        except HTTPError as error:
            body = error.read().decode(errors="replace")
            raise GitHubAPIError(error.code, body) from error
        except URLError as error:
            raise RuntimeError(f"Asset upload failed: {error.reason}") from error


CHANGELOG_SECTIONS = (
    ("feat", "### Features"),
    ("fix", "### Fixes"),
    ("", "### Other changes"),
)
CONVENTIONAL_PREFIX = re.compile(r"^(?P<type>[a-z]+)(?:\([^)]*\))?!?:\s*(?P<subject>.+)$")

# Hand-written release notes live under releases/<tag>.yaml (preferred over commits).
REPO_ROOT = Path(__file__).resolve().parents[2]
NOTES_YAML_KEYS = ("features", "fixes", "changes", "breaking")
NOTES_YAML_HEADINGS = {
    "features": "### Features",
    "fixes": "### Fixes",
    "changes": "### Changes",
    "breaking": "### Breaking changes",
}


def git_output(*args: str) -> str:
    """Run a git command and return its stdout."""
    result = subprocess.run(
        ["git", *args],
        capture_output=True,
        text=True,
        check=True,
        timeout=60,
    )
    return result.stdout


def previous_semver_tag(tag: str) -> str | None:
    """Return the SemVer tag immediately before `tag` (or the latest if tag is new)."""
    tags = [
        line.strip()
        for line in git_output("tag", "--sort=-v:refname").splitlines()
        if TAG_PATTERN.fullmatch(line.strip())
    ]
    if tag in tags:
        newer = tags[tags.index(tag) + 1 :]
        return newer[0] if newer else None
    return tags[0] if tags else None


def latest_semver_tag() -> str | None:
    """Return the highest SemVer tag, or None."""
    tags = [
        line.strip()
        for line in git_output("tag", "--sort=-v:refname").splitlines()
        if TAG_PATTERN.fullmatch(line.strip())
    ]
    return tags[0] if tags else None


def next_patch_tag() -> str:
    """Return the next vMAJOR.MINOR.PATCH after the latest tag (or v0.0.1)."""
    last = latest_semver_tag()
    if not last:
        return "v0.0.1"
    core = last.lstrip("v").split("-", 1)[0].split("+", 1)[0]
    major, minor, patch = (int(part) for part in core.split("."))
    return f"v{major}.{minor}.{patch + 1}"


def changelog_commits(tag: str) -> tuple[str | None, list[str]]:
    """Return the previous SemVer tag and commit subjects since it.

    When `tag` already exists, the range ends at the tag; otherwise (a
    workflow_dispatch run that names a new tag) it ends at HEAD.
    """
    tags = [
        line.strip()
        for line in git_output("tag", "--sort=-v:refname").splitlines()
        if TAG_PATTERN.fullmatch(line.strip())
    ]
    if tag in tags:
        end = tag
        newer = tags[tags.index(tag) + 1 :]
        previous = newer[0] if newer else None
    else:
        end = "HEAD"
        previous = tags[0] if tags else None

    log_range = f"{previous}..{end}" if previous else end
    subjects = [
        line.strip()
        for line in git_output(
            "log", "--no-merges", "--pretty=format:%s", log_range
        ).splitlines()
        if line.strip()
    ]
    return previous, subjects


def notes_path_for_tag(tag: str, root: Path | None = None) -> Path | None:
    """Return releases/<tag>.yaml or .yml if it exists."""
    base = root if root is not None else REPO_ROOT
    for name in (f"{tag}.yaml", f"{tag}.yml"):
        path = base / "releases" / name
        if path.is_file():
            return path
    return None


def _unquote_yaml_scalar(value: str) -> str:
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
        return value[1:-1]
    return value


def parse_release_notes_yaml(text: str) -> dict[str, Any]:
    """Parse the restricted release-notes YAML dialect (stdlib only).

    Supported:
      key: scalar
      key: |
        multiline
      key:
        - list item
        - "quoted: item"
      # comments and blank lines
    """
    result: dict[str, Any] = {}
    lines = text.splitlines()
    i = 0
    n = len(lines)

    while i < n:
        raw = lines[i]
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            i += 1
            continue

        key_match = re.match(r"^([A-Za-z_][\w]*)\s*:\s*(.*)$", raw)
        if not key_match:
            i += 1
            continue

        key = key_match.group(1)
        val = key_match.group(2).rstrip()
        i += 1

        if val in ("|", ">"):
            block: list[str] = []
            while i < n:
                nxt = lines[i]
                if not nxt.strip():
                    block.append("")
                    i += 1
                    continue
                if nxt.startswith("#") and not nxt.startswith(" "):
                    break
                if nxt.startswith(" ") or nxt.startswith("\t"):
                    if nxt.startswith("  "):
                        block.append(nxt[2:])
                    else:
                        block.append(nxt.lstrip())
                    i += 1
                    continue
                break
            while block and block[-1] == "":
                block.pop()
            if val == "|":
                result[key] = "\n".join(block)
            else:
                result[key] = " ".join(part.strip() for part in block if part.strip())
            continue

        if val != "":
            result[key] = _unquote_yaml_scalar(val)
            continue

        items: list[str] = []
        while i < n:
            nxt = lines[i]
            if not nxt.strip() or nxt.strip().startswith("#"):
                i += 1
                continue
            item_match = re.match(r"^\s*-\s+(.*)$", nxt)
            if not item_match:
                break
            items.append(_unquote_yaml_scalar(item_match.group(1)))
            i += 1
        result[key] = items

    return result


def load_release_notes_file(path: Path) -> dict[str, Any]:
    """Load release notes from a YAML file."""
    text = path.read_text(encoding="utf-8")
    try:
        import yaml  # type: ignore

        data = yaml.safe_load(text)
        if isinstance(data, dict):
            return data
    except Exception:
        pass
    return parse_release_notes_yaml(text)


def build_release_body_from_notes(
    data: dict[str, Any],
    previous: str | None,
    tag: str,
    repository: str,
) -> str:
    """Build Markdown release notes from a releases/<tag>.yaml document."""
    if data.get("tag") not in (None, "", tag):
        raise ValueError(
            f"releases notes tag {data.get('tag')!r} does not match release tag {tag!r}"
        )

    title = str(data.get("title") or "What's new").strip() or "What's new"
    lines = [f"## {title}"]

    highlights = data.get("highlights")
    if isinstance(highlights, str) and highlights.strip():
        lines.append("")
        lines.append(highlights.strip())

    for key in NOTES_YAML_KEYS:
        items = data.get(key)
        if not items:
            continue
        if isinstance(items, str):
            items = [items]
        if not isinstance(items, list):
            continue
        cleaned = [str(item).strip() for item in items if str(item).strip()]
        if not cleaned:
            continue
        lines.append("")
        lines.append(NOTES_YAML_HEADINGS[key])
        lines.extend(f"- {item}" for item in cleaned)

    prev = data.get("previous")
    if isinstance(prev, str) and prev.strip():
        previous = prev.strip()

    if previous:
        lines.append("")
        lines.append(
            f"**Full changelog**: https://github.com/{repository}/compare/{previous}...{tag}"
        )

    body = "\n".join(lines).rstrip() + "\n"
    if body.strip() == f"## {title}":
        raise ValueError(f"release notes file for {tag} has no content sections")
    return body


def build_release_body(
    subjects: list[str],
    previous: str | None,
    tag: str,
    repository: str,
) -> str | None:
    """Build Markdown release notes from conventional commit subjects."""
    if not subjects:
        return None

    grouped: dict[str, list[str]] = {prefix: [] for prefix, _ in CHANGELOG_SECTIONS}
    for subject in subjects:
        matched = CONVENTIONAL_PREFIX.match(subject)
        commit_type = matched.group("type") if matched else ""
        text = matched.group("subject") if matched else subject
        bucket = commit_type if commit_type in dict(CHANGELOG_SECTIONS) else ""
        grouped[bucket].append(text)

    lines = ["## What's new"]
    for prefix, heading in CHANGELOG_SECTIONS:
        if not grouped[prefix]:
            continue
        lines.append("")
        lines.append(heading)
        lines.extend(f"- {text}" for text in grouped[prefix])

    if previous:
        lines.append("")
        lines.append(
            f"**Full changelog**: https://github.com/{repository}/compare/{previous}...{tag}"
        )
    return "\n".join(lines) + "\n"


def release_notes(tag: str, repository: str, root: Path | None = None) -> str | None:
    """Build release notes: prefer releases/<tag>.yaml, else conventional commits."""
    notes_file = notes_path_for_tag(tag, root=root)
    if notes_file is not None:
        try:
            data = load_release_notes_file(notes_file)
            try:
                previous = previous_semver_tag(tag)
            except (OSError, subprocess.SubprocessError):
                previous = None
            body = build_release_body_from_notes(data, previous, tag, repository)
            try:
                display = notes_file.relative_to(root or REPO_ROOT)
            except ValueError:
                display = notes_file
            print(f"Using release notes from {display}")
            return body
        except (OSError, ValueError, TypeError) as error:
            print(
                f"WARNING: could not load release notes from {notes_file}: {error}",
                file=sys.stderr,
            )

    try:
        previous, subjects = changelog_commits(tag)
        return build_release_body(subjects, previous, tag, repository)
    except (OSError, subprocess.SubprocessError) as error:
        print(f"WARNING: could not build changelog from commits: {error}", file=sys.stderr)
        return None


def validate_tag(tag: str) -> str:
    """Validate and return a SemVer release tag."""
    if not TAG_PATTERN.fullmatch(tag):
        raise ValueError(
            f"invalid release tag {tag!r}; expected SemVer such as v1.2.3 or v1.2.3-rc.1"
        )
    return tag


def sha256(path: Path) -> str:
    """Calculate a file's SHA-256 checksum."""
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def collect_and_verify_assets(release_dir: Path) -> list[Path]:
    """Collect the expected release files and verify checksums."""
    if not release_dir.is_dir():
        raise FileNotFoundError(f"release directory not found: {release_dir}")

    binary_paths = {path.name: path for path in release_dir.glob("confctl-*")}
    missing = EXPECTED_BINARIES - binary_paths.keys()
    unexpected = binary_paths.keys() - EXPECTED_BINARIES
    if missing:
        raise RuntimeError(f"missing release binaries: {', '.join(sorted(missing))}")
    if unexpected:
        raise RuntimeError(f"unexpected release binaries: {', '.join(sorted(unexpected))}")

    checksums_path = release_dir / "checksums.txt"
    if not checksums_path.is_file():
        raise FileNotFoundError(f"checksum file not found: {checksums_path}")

    checksums: dict[str, str] = {}
    for line_number, line in enumerate(checksums_path.read_text().splitlines(), start=1):
        parts = line.split()
        if len(parts) != 2:
            raise RuntimeError(f"invalid checksums.txt line {line_number}: {line!r}")
        checksum, filename = parts
        checksums[filename.removeprefix("*")] = checksum.lower()

    if checksums.keys() != EXPECTED_BINARIES:
        raise RuntimeError("checksums.txt does not contain exactly the expected binaries")

    for filename, path in binary_paths.items():
        actual = sha256(path)
        if actual != checksums[filename]:
            raise RuntimeError(
                f"checksum mismatch for {filename}: expected {checksums[filename]}, got {actual}"
            )

    return [binary_paths[name] for name in sorted(EXPECTED_BINARIES)] + [
        checksums_path
    ]


def create_or_get_release(
    api: GitHubAPI,
    repository: str,
    tag: str,
    target_commitish: str,
    body: str | None = None,
) -> dict[str, Any]:
    """Return an existing release or create a new one."""
    releases_url = f"{api.api_url}/repos/{repository}/releases"
    tag_url = f"{releases_url}/tags/{quote(tag, safe='')}"
    try:
        existing = api.request_json("GET", tag_url)
        if not isinstance(existing, dict):
            raise RuntimeError("GitHub returned an invalid release response")
        print(f"Using existing release: {tag}")
        # Refresh body when we have better notes than an empty/placeholder body.
        if body and existing.get("id"):
            try:
                api.request_json(
                    "PATCH",
                    f"{releases_url}/{existing['id']}",
                    {"body": body, "name": tag},
                )
                print(f"Updated release notes for: {tag}")
            except GitHubAPIError as error:
                print(f"WARNING: could not update release body: {error}", file=sys.stderr)
        return existing
    except GitHubAPIError as error:
        if error.status != 404:
            raise

    prerelease = "-" in tag.partition("+")[0]
    payload: dict[str, Any] = {
        "tag_name": tag,
        "target_commitish": target_commitish,
        "name": tag,
        "draft": False,
        "prerelease": prerelease,
        "generate_release_notes": body is None,
    }
    if body is not None:
        payload["body"] = body
    created = api.request_json("POST", releases_url, payload)
    if not isinstance(created, dict):
        raise RuntimeError("GitHub returned an invalid release response")
    print(f"Created release: {tag}")
    return created


def replace_assets(
    api: GitHubAPI,
    repository: str,
    release: dict[str, Any],
    assets: list[Path],
) -> None:
    """Replace assets with matching names and upload the current files."""
    expected_names = {asset.name for asset in assets}
    for existing in release.get("assets", []):
        if existing.get("name") not in expected_names:
            continue
        asset_id = existing.get("id")
        print(f"Deleting existing asset: {existing['name']}")
        api.request_json(
            "DELETE",
            f"{api.api_url}/repos/{repository}/releases/assets/{asset_id}",
        )

    upload_url = release.get("upload_url")
    if not isinstance(upload_url, str):
        raise RuntimeError("GitHub release response does not include upload_url")

    for asset in assets:
        print(f"Uploading {asset.name} ({asset.stat().st_size:,} bytes)...")
        uploaded = api.upload_asset(upload_url, asset)
        print(f"Uploaded: {uploaded.get('browser_download_url', asset.name)}")


def update_version_file(tag: str, token: str, repository: str) -> None:
    """Write VERSION (and optionally Cargo.toml) version and push to main."""
    version_str = tag.lstrip("v").split("-", 1)[0].split("+", 1)[0]
    print(f"Updating VERSION file to: {version_str}")

    subprocess.run(
        ["git", "config", "user.name", "github-actions"],
        check=True,
        capture_output=True,
    )
    subprocess.run(
        ["git", "config", "user.email", "github-actions@github.com"],
        check=True,
        capture_output=True,
    )
    subprocess.run(["git", "fetch", "origin", "main"], check=True, capture_output=True)
    subprocess.run(
        ["git", "checkout", "-B", "main", "origin/main"],
        check=True,
        capture_output=True,
    )

    Path("VERSION").write_text(version_str + "\n", encoding="utf-8")

    cargo = Path("Cargo.toml")
    if cargo.is_file():
        text = cargo.read_text(encoding="utf-8")
        updated, n = re.subn(
            r'(?m)^version\s*=\s*"[^"]*"',
            f'version = "{version_str}"',
            text,
            count=1,
        )
        if n:
            cargo.write_text(updated, encoding="utf-8")

    subprocess.run(["git", "add", "VERSION", "Cargo.toml"], check=False, capture_output=True)
    commit = subprocess.run(
        ["git", "commit", "-m", f"chore(release): set VERSION to {version_str}"],
        capture_output=True,
        text=True,
    )
    if commit.returncode != 0:
        print("No version changes to commit; skipping")
        return

    push_url = f"https://x-access-token:{token}@github.com/{repository}.git"
    subprocess.run(
        ["git", "push", push_url, "HEAD:main"],
        check=True,
        capture_output=True,
    )
    print(f"Pushed VERSION={version_str} to main")


def resolve_release_tag() -> str:
    """Pick the tag to publish from env (RELEASE_TAG, ref name, or latest)."""
    explicit = os.environ.get("RELEASE_TAG", "").strip()
    if explicit:
        return validate_tag(explicit)

    ref_type = os.environ.get("GITHUB_REF_TYPE", "").strip()
    ref_name = os.environ.get("GITHUB_REF_NAME", "").strip()
    update_latest = os.environ.get("UPDATE_LATEST_RELEASE", "").lower() in (
        "1",
        "true",
        "yes",
    )

    if ref_type == "tag" and ref_name:
        return validate_tag(ref_name)

    if update_latest or ref_type == "branch":
        latest = latest_semver_tag()
        if not latest:
            raise RuntimeError("no SemVer tags found; create v0.0.1 first")
        print(f"Using latest tag for asset refresh: {latest}")
        return latest

    if ref_name and TAG_PATTERN.fullmatch(ref_name):
        return validate_tag(ref_name)

    raise RuntimeError(
        "RELEASE_TAG or a SemVer GITHUB_REF_NAME is required "
        "(or set UPDATE_LATEST_RELEASE=true on a branch push)"
    )


def publish_release() -> None:
    """Validate the environment and publish all release assets."""
    repository = os.environ.get("GITHUB_REPOSITORY", "").strip()
    token = os.environ.get("GITHUB_TOKEN", "").strip()
    api_url = os.environ.get("GITHUB_API_URL", "https://api.github.com")
    target_commitish = os.environ.get("GITHUB_SHA", "").strip() or "main"
    sync_version = os.environ.get("CONFCTL_SYNC_VERSION", "true").lower() in (
        "1",
        "true",
        "yes",
    )

    if not repository:
        raise RuntimeError("GITHUB_REPOSITORY is required")
    if not token:
        raise RuntimeError("GITHUB_TOKEN is required")

    tag = resolve_release_tag()
    assets = collect_and_verify_assets(Path("release"))
    body = release_notes(tag, repository)
    api = GitHubAPI(token, api_url)
    release = create_or_get_release(api, repository, tag, target_commitish, body)
    replace_assets(api, repository, release, assets)

    if sync_version and os.environ.get("GITHUB_REF_TYPE", "") == "tag":
        try:
            update_version_file(tag, token, repository)
        except (OSError, subprocess.SubprocessError, RuntimeError) as error:
            print(f"WARNING: could not sync VERSION on main: {error}", file=sys.stderr)

    print(f"Release published successfully: {tag}")


def bump_and_push_tag() -> str:
    """Create the next patch tag and push it to origin (tag.yml)."""
    repository = os.environ.get("GITHUB_REPOSITORY", "").strip()
    token = os.environ.get("GITHUB_TOKEN", "").strip()
    if not repository:
        raise RuntimeError("GITHUB_REPOSITORY is required")
    if not token:
        raise RuntimeError("GITHUB_TOKEN is required")

    subprocess.run(["git", "fetch", "--prune", "--tags"], check=True, capture_output=True)
    tag = next_patch_tag()
    validate_tag(tag)
    print(f"Creating tag: {tag}")
    subprocess.run(["git", "tag", tag], check=True, capture_output=True)
    push_url = f"https://x-access-token:{token}@github.com/{repository}.git"
    subprocess.run(["git", "push", push_url, tag], check=True, capture_output=True)
    print(f"Tag {tag} pushed successfully")
    return tag


def parse_args() -> argparse.Namespace:
    """Parse command-line arguments."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--validate-tag",
        metavar="TAG",
        help="validate a release tag without contacting GitHub",
    )
    parser.add_argument(
        "--bump-tag",
        action="store_true",
        help="create and push the next patch SemVer tag (used by tag.yml)",
    )
    return parser.parse_args()


def main() -> int:
    """Run the release publisher."""
    args = parse_args()
    try:
        if args.validate_tag:
            print(validate_tag(args.validate_tag))
        elif args.bump_tag:
            bump_and_push_tag()
        else:
            publish_release()
    except (GitHubAPIError, OSError, RuntimeError, ValueError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
