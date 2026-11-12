#!/usr/bin/env python3
"""Fetches Minecraft 26.3's manifest, jars and assets for `horizon-prepare` (IOS.md).

Horizon never ships Minecraft's files: this downloads them from Mojang's own servers into a
local `.minecraft` folder, which `cargo run -p horizon-prepare -- --prepare --minecraft-dir
.minecraft --server-jar server.jar` then turns into the resources the app carries.

    python3 packaging/ios/fetch_assets.py            # -> .minecraft/ and server.jar
"""
import json
import os
import sys
import urllib.request
from concurrent.futures import ThreadPoolExecutor

VERSION = "26.3"
# Mojang's version manifest entry for 26.3.
VERSION_URL = "https://piston-meta.mojang.com/v1/packages/80608ff6753705b6a82e2cb86a5937d4d233abd4/26.3.json"
SERVER_URL = "https://piston-data.mojang.com/v1/objects/33680f5f2ac32864d6d7cf5e56a705fdb3e05f4c/server.jar"
HEADERS = {"User-Agent": "Mozilla/5.0 (Horizon iOS packaging)"}


def fetch(url: str, path: str) -> None:
    """Downloads `url` to `path` unless it is already there (an interrupted run resumes)."""
    if os.path.exists(path) and os.path.getsize(path) > 0:
        return
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    request = urllib.request.Request(url, headers=HEADERS)
    with urllib.request.urlopen(request) as response, open(path + ".part", "wb") as out:
        out.write(response.read())
    os.replace(path + ".part", path)


def read_json(url: str, path: str) -> dict:
    fetch(url, path)
    with open(path, "rb") as f:
        return json.load(f)


def main() -> int:
    minecraft = sys.argv[1] if len(sys.argv) > 1 else ".minecraft"
    version_dir = os.path.join(minecraft, "versions", VERSION)

    print(f"[+] {VERSION} manifest")
    version = read_json(VERSION_URL, os.path.join(version_dir, f"{VERSION}.json"))

    print(f"[+] {VERSION} client and server jars")
    fetch(version["downloads"]["client"]["url"], os.path.join(version_dir, f"{VERSION}.jar"))
    fetch(SERVER_URL, "server.jar")

    index_id = version["assetIndex"]["id"]
    print(f"[+] asset index {index_id}")
    index_path = os.path.join(minecraft, "assets", "indexes", f"{index_id}.json")
    index = read_json(version["assetIndex"]["url"], index_path)

    libraries = os.path.join(minecraft, "libraries")
    objects = os.path.join(minecraft, "assets", "objects")
    tasks = []
    for library in version.get("libraries", []):
        artifact = library.get("downloads", {}).get("artifact")
        if artifact:
            tasks.append((artifact["url"], os.path.join(libraries, artifact["path"])))
    for obj in index.get("objects", {}).values():
        digest = obj["hash"]
        tasks.append(
            (
                f"https://resources.download.minecraft.net/{digest[:2]}/{digest}",
                os.path.join(objects, digest[:2], digest),
            )
        )

    print(f"[+] {len(tasks)} libraries and assets")
    with ThreadPoolExecutor(max_workers=20) as pool:
        for error in pool.map(lambda task: _try_fetch(task), tasks):
            if error:
                print(f"warning: {error}", file=sys.stderr)
    print("[+] done: now run `cargo run --release -p horizon-prepare -- --prepare "
          f"--minecraft-dir {minecraft} --server-jar server.jar`")
    return 0


def _try_fetch(task: tuple[str, str]) -> str | None:
    """One download; a missing library or asset is a warning, not the end of the run."""
    url, path = task
    try:
        fetch(url, path)
        return None
    except Exception as error:  # noqa: BLE001 - report and keep going, as the APK build does
        return f"{url}: {error}"


if __name__ == "__main__":
    raise SystemExit(main())
