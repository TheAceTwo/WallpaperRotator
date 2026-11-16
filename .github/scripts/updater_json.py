#!/usr/bin/env python3
"""Build latest.json (the auto-update manifest) for a GitHub release.

tauri-action writes this itself, but it looks for assets under their
original file names. GitHub renames uploads with characters like ’ in them
("Cody’s" -> "Cody.s"), so tauri-action can't find the signatures and skips
the manifest. This script reads the names GitHub actually stored.

Usage: updater_json.py <owner/repo> <tag> <output-path>
Needs the `gh` CLI (authenticated) on PATH.
"""
import json
import subprocess
import sys
from datetime import datetime, timezone

# Updater platform keys -> file-name ending of the matching update bundle.
PLATFORMS = {
    "windows-x86_64": "_x64-setup.exe",
    "windows-x86_64-nsis": "_x64-setup.exe",
    "windows-x86_64-msi": "_x64_en-US.msi",
    "windows-aarch64": "_arm64-setup.exe",
    "windows-aarch64-nsis": "_arm64-setup.exe",
    "darwin-aarch64": "_universal.app.tar.gz",
    "darwin-x86_64": "_universal.app.tar.gz",
    "darwin-aarch64-app": "_universal.app.tar.gz",
    "darwin-x86_64-app": "_universal.app.tar.gz",
}


def gh(*args, binary=False):
    out = subprocess.run(["gh", "api", *args], check=True, capture_output=True)
    return out.stdout if binary else json.loads(out.stdout)


def main(repo, tag, out_path):
    releases = gh(f"repos/{repo}/releases?per_page=30")
    release = next((r for r in releases if r["tag_name"] == tag), None)
    if release is None:
        sys.exit(f"no release (draft or published) with tag {tag}")
    assets = {a["name"]: a for a in release["assets"]}

    platforms = {}
    for key, ending in PLATFORMS.items():
        bundle = next((n for n in assets if n.endswith(ending)), None)
        sig = assets.get(f"{bundle}.sig") if bundle else None
        if not bundle or not sig:
            print(f"  skip {key}: no {ending} bundle with a .sig")
            continue
        signature = gh(
            "-H", "Accept: application/octet-stream", f"repos/{repo}/releases/assets/{sig['id']}", binary=True
        ).decode().strip()
        # The tag URL keeps working after the draft is published.
        url = f"https://github.com/{repo}/releases/download/{tag}/{bundle}"
        platforms[key] = {"signature": signature, "url": url}
        print(f"  {key}: {bundle}")

    if not platforms:
        sys.exit("no signed update bundles found")
    manifest = {
        "version": tag.removeprefix("v"),
        "notes": f"See https://github.com/{repo}/releases/tag/{tag}",
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }
    with open(out_path, "w") as f:
        json.dump(manifest, f, indent=2)
    print(f"wrote {out_path} with {len(platforms)} platforms")


if __name__ == "__main__":
    main(*sys.argv[1:4])
