#!/usr/bin/env python3
"""Script to automatically create releases on GitHub."""

from typing import Optional
from pathlib import Path
import subprocess
import requests
import tomllib
import shutil
import sys
import os


class GitHubReleaser:
    """Manages GitHub release creation."""

    def __init__(self):
        self.server = os.getenv("GITHUB_SERVER_URL", "")
        self.repo = os.getenv("GITHUB_REPOSITORY", "")
        self.ref_type = os.getenv("GITHUB_REF_TYPE", "")
        self.ref = os.getenv("GITHUB_REF", "")
        self.token = os.getenv("GITHUB_TOKEN", "")
        self.binary_name = "confctl"
        self.update_latest = os.getenv("UPDATE_LATEST_RELEASE", "").lower() in (
            "1",
            "true",
            "yes",
        )

    def run_command(
        self,
        cmd: str,
        capture_output: bool = False,
    ) -> Optional[str]:
        """Executes a shell command."""
        try:
            if capture_output:
                result = subprocess.check_output(cmd, shell=True, text=True)
                return result.strip()
            subprocess.check_call(cmd, shell=True)
            return None
        except subprocess.CalledProcessError as e:
            print(f"ERROR: Command failed: {cmd}")
            raise e

    def print_debug_info(self):
        """Prints debug information."""
        print(f"DEBUG: Server: {self.server}")
        print(f"DEBUG: Repo: {self.repo}")
        print(f"DEBUG: Ref type: {self.ref_type}")
        print(f"DEBUG: Ref: {self.ref}")

    def get_next_version(self) -> str:
        """Calculates the next version based on existing tags."""
        self.run_command("git fetch --prune --tags")
        last_tag = self.run_command(
            "git tag --sort=-v:refname | head -n1", capture_output=True
        )

        if not last_tag:
            last_tag = "v0.0.0"

        print(f"Last tag: {last_tag}")

        version_str = last_tag.lstrip("v")
        major, minor, patch = map(int, version_str.split("."))

        new_tag = f"v{major}.{minor}.{patch + 1}"
        print(f"Creating new tag: {new_tag}")

        return new_tag

    def get_latest_tag(self) -> str:
        """Return the latest tag (highest semver). Creates v0.0.1 if none exist."""
        self.run_command("git fetch --prune --tags")
        last_tag = self.run_command(
            "git tag --sort=-v:refname | head -n1", capture_output=True
        )
        if not last_tag:
            print("No tags found; bootstrapping first release tag v0.0.1")
            self.push_tag("v0.0.1")
            return "v0.0.1"
        print(f"Latest tag: {last_tag}")
        return last_tag

    def push_tag(self, tag: str):
        """Creates and pushes a new tag to the repository."""
        self.run_command(f"git tag {tag}")

        if self.server == "https://github.com":
            push_url = f"https://x-access-token:{self.token}@github.com/{self.repo}.git"
            self.run_command(f"git push {push_url} {tag}")
        else:
            self.run_command(f"git push origin {tag}")

        print(f"Tag {tag} pushed successfully")

    def handle_branch_push(self) -> str:
        """Handles a branch push by creating a new tag."""
        print("Branch push detected, creating new tag...")
        tag = self.get_next_version()
        self.push_tag(tag)
        print(f"Tag {tag} pushed, exiting to let tag trigger handle the release")
        sys.exit(0)

    def extract_tag_from_ref(self) -> str:
        """Extracts the tag name from the ref."""
        return self.ref.rsplit("/", 1)[-1]

    def build_binary(self, tag: str):
        """Builds the Rust binary with release profile."""
        print(f"Building binary for tag: {tag}")

        build_cmd = "cargo build --release --target x86_64-unknown-linux-musl"
        self.run_command(build_cmd)

        binary_path = Path(
            f"target/x86_64-unknown-linux-musl/release/{self.binary_name}"
        )
        if not binary_path.exists():
            binary_path = Path(f"target/release/{self.binary_name}")
            if not binary_path.exists():
                print(f"ERROR: Binary {self.binary_name} not found after build")
                sys.exit(1)

        file_size = binary_path.stat().st_size
        print(f"Binary built successfully: {file_size:,} bytes")

        shutil.copy2(str(binary_path), self.binary_name)

        return file_size

    def update_version_file(self, tag: str):
        """Updates the VERSION file to match the release tag and pushes to main."""
        version_str = tag.lstrip("v")
        print(f"Updating VERSION file to: {version_str}")

        self.run_command("git config user.name 'github-actions'", capture_output=False)
        self.run_command(
            "git config user.email 'github-actions@github.com'",
            capture_output=False,
        )

        self.run_command("git fetch origin main", capture_output=False)
        self.run_command("git checkout -B main origin/main", capture_output=False)

        Path("VERSION").write_text(version_str + "\n", encoding="utf-8")

        self.run_command("git add VERSION", capture_output=False)
        try:
            self.run_command(
                f"git commit -m 'chore(release): set VERSION to {version_str}'",
                capture_output=False,
            )
        except subprocess.CalledProcessError:
            print("No changes to VERSION; skipping commit")
            return

        if self.server == "https://github.com":
            push_url = f"https://x-access-token:{self.token}@github.com/{self.repo}.git"
            self.run_command("git push %s HEAD:main" % push_url, capture_output=False)
        else:
            self.run_command("git push origin HEAD:main", capture_output=False)

    def create_or_get_release(self, tag: str) -> dict:
        """Creates or retrieves an existing release on GitHub."""
        api_url = f"https://api.github.com/repos/{self.repo}/releases"
        headers = {
            "Authorization": f"Bearer {self.token}",
            "Accept": "application/vnd.github+json",
        }

        body = ""
        toml_title = None
        if Path("releases_notes.toml").exists():
            try:
                with open("releases_notes.toml", "rb") as f:
                    data = tomllib.load(f)
                releases = data.get("releases", {}) if isinstance(data, dict) else {}
                version_str = tag.lstrip("v")
                candidates = [
                    tag,
                    version_str,
                    tag.replace(".", "-"),
                    version_str.replace(".", "-"),
                ]
                entry = None
                for key in candidates:
                    if isinstance(releases, dict) and key in releases:
                        entry = releases.get(key)
                        break
                if isinstance(entry, dict):
                    toml_title = (entry.get("title") or "").strip()
                    mapped_body = entry.get("body", "").strip()
                    if mapped_body:
                        body = mapped_body
            except Exception as e:
                print(f"WARNING: Could not parse releases_notes.toml: {e}")
        if not body:
            body = os.getenv("RELEASE_BODY", "").strip()
        if not body:
            try:
                body = (
                    self.run_command(
                        f"git tag -l --format='%(contents)' {tag}", capture_output=True
                    )
                    or ""
                )
            except Exception:
                body = ""
        if not body:
            try:
                last_tag = self.run_command(
                    "git tag --sort=-v:refname | sed -n '2p'", capture_output=True
                )
                if last_tag:
                    body = self.run_command(
                        f"git log --pretty=format:'- %s (%h)' {last_tag}..HEAD",
                        capture_output=True,
                    )
                else:
                    body = self.run_command(
                        "git log --pretty=format:'- %s (%h)'", capture_output=True
                    )
            except Exception:
                body = ""

        if toml_title:
            body = (toml_title + "\n\n" + (body or "")).strip()

        release_data = {
            "tag_name": tag,
            "name": tag,
            "body": (body or f"Automated release for {tag}"),
            "draft": False,
            "prerelease": False,
        }

        print(f"Creating release for tag: {tag}")
        response = requests.post(api_url, headers=headers, json=release_data)

        if response.status_code == 422:
            print("Release already exists, fetching existing release...")
            response = requests.get(f"{api_url}/tags/{tag}", headers=headers)
        elif response.status_code != 201:
            print(
                f"ERROR: Failed to create release. "
                f"Status: {response.status_code}, Response: {response.text}"
            )
            sys.exit(1)

        if response.status_code not in (200, 201):
            print(
                f"ERROR: Failed to get release info. "
                f"Status: {response.status_code}, Response: {response.text}"
            )
            sys.exit(1)

        return response.json()

    def upload_binary(self, release_data: dict, file_size: int):
        """Uploads the binary to the release."""
        upload_url_template = release_data["upload_url"]
        upload_url = upload_url_template.replace(
            "{?name,label}", f"?name={self.binary_name}"
        )

        print(f"Release ID: {release_data['id']}")
        print(f"Uploading binary ({file_size:,} bytes)...")

        headers = {
            "Authorization": f"Bearer {self.token}",
            "Content-Type": "application/octet-stream",
            "Accept": "application/vnd.github+json",
        }

        assets_api = release_data.get("assets_url")
        if assets_api:
            list_headers = {
                "Authorization": f"Bearer {self.token}",
                "Accept": "application/vnd.github+json",
            }
            try:
                resp = requests.get(assets_api, headers=list_headers)
                if resp.status_code == 200:
                    for asset in resp.json():
                        if asset.get("name") == self.binary_name:
                            asset_id = asset.get("id")
                            del_url = f"https://api.github.com/repos/{self.repo}/releases/assets/{asset_id}"
                            print(
                                f"Deleting existing asset '{self.binary_name}' (id={asset_id})"
                            )
                            requests.delete(del_url, headers=list_headers)
                else:
                    print(f"WARNING: Could not list assets (status {resp.status_code})")
            except Exception as e:
                print(f"WARNING: Could not delete existing asset: {e}")

        with open(self.binary_name, "rb") as binary_file:
            response = requests.post(
                upload_url, headers=headers, data=binary_file.read()
            )

        if response.status_code == 201:
            print("✓ Binary uploaded successfully!")
            asset_info = response.json()
            print(f"Asset URL: {asset_info['browser_download_url']}")
        else:
            print(
                f"ERROR: Failed to upload binary. "
                f"Status: {response.status_code}\nResponse: {response.text}"
            )
            sys.exit(1)

    def run(self):
        """Runs the complete release process."""
        self.print_debug_info()

        if self.ref_type == "branch":
            if self.update_latest:
                tag = self.get_latest_tag()
                print(f"Branch push: updating release for {tag}")
                try:
                    self.update_version_file(tag)
                except Exception as e:
                    print(f"WARNING: Could not update VERSION on main: {e}")
            else:
                print(
                    "Branch push detected; UPDATE_LATEST_RELEASE is not set. Skipping release."
                )
                sys.exit(0)
        elif self.ref_type == "tag":
            tag = self.extract_tag_from_ref()
            print(f"Tag push detected: {tag}")
            try:
                self.update_version_file(tag)
            except Exception as e:
                print(f"WARNING: Could not update VERSION on main: {e}")
        else:
            print(f"Unknown ref type: {self.ref_type}")
            sys.exit(1)

        file_size = self.build_binary(tag)

        if self.server != "https://github.com":
            print("Gitea detected; skipping release upload")
            sys.exit(0)

        print("Creating or updating GitHub release...")
        release_data = self.create_or_get_release(tag)
        self.upload_binary(release_data, file_size)

        print(f"Release created successfully: {tag}")


def main():
    """Entry point of the script."""
    releaser = GitHubReleaser()
    releaser.run()


if __name__ == "__main__":
    main()
