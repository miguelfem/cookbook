"""Bundle only source assets and the pinned runtime; no mutable state or secrets."""
from pathlib import Path
import os
import struct
import shutil
import subprocess
ROOT = Path(__file__).resolve().parents[2]
APP = Path(__file__).resolve().parent
if os.name == "nt":
    raise SystemExit("CookBook supports macOS/Linux; Windows is excluded.")
host = subprocess.check_output(["rustc", "--print", "host-tuple"], text=True).strip()
target = os.environ.get("TAURI_ENV_TARGET_TRIPLE", host)
allowed = {"aarch64-apple-darwin", "x86_64-apple-darwin", "universal-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"}
if target not in allowed:
    raise SystemExit("Unsupported desktop target: " + target)
target_dir = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target")))
include_jev = os.environ.get("COOKBOOK_INCLUDE_JEV", "0")
if include_jev not in {"0", "1"}:
    raise SystemExit("COOKBOOK_INCLUDE_JEV must be 0 or 1; use tools/build_desktop_local.py --with-jev for the optional build.")
if not target_dir.is_absolute():
    target_dir = ROOT / target_dir
binaries = APP / "src-tauri/binaries"
binaries.mkdir(exist_ok=True)
def runtime(arch):
    # Explicit target keeps sidecar and desktop architectures identical.
    features = ["--features", "jev"] if include_jev == "1" else []
    subprocess.run(["cargo", "build", "--release", "--locked", "--target", arch, "-p", "apr-cli", "--bin", "apr", *features], cwd=ROOT, check=True)
    return target_dir / arch / "release/apr"
if target == "universal-apple-darwin":
    arm, intel = runtime("aarch64-apple-darwin"), runtime("x86_64-apple-darwin")
    subprocess.run(["lipo", "-create", str(arm), str(intel), "-output", str(binaries / f"apr-{target}")], check=True)
else:
    shutil.copy2(runtime(target), binaries / f"apr-{target}")
# Ship native sizes so Finder and the Dock use the same cookbook identity.
chunks = []
for size, kind in [(16, b"icp4"), (32, b"icp5"), (64, b"icp6"), (128, b"ic07"),
                   (256, b"ic08"), (512, b"ic09"), (1024, b"ic10")]:
    name = "icon.png" if size == 512 else f"icon-{size}.png"
    png = (APP / "src-tauri/icons" / name).read_bytes()
    if png[:8] != b"\x89PNG\r\n\x1a\n" or struct.unpack(">II", png[16:24]) != (size, size):
        raise SystemExit(f"Invalid {size}px app icon: {name}")
    chunks.append(kind + struct.pack(">I", len(png) + 8) + png)
payload = b"".join(chunks)
(APP / "src-tauri/icons/icon.icns").write_bytes(b"icns" + struct.pack(">I", len(payload) + 8) + payload)
dist = APP / "dist"
dist.mkdir(exist_ok=True)
assets = ROOT / "crates/apr-cli/assets"
for name in ["inspector.css", "inspector.js"]:
    shutil.copy2(assets / name, dist / name)
html = (assets / "inspector.html").read_text().replace("%APR_MODE%", "author")
html = html.replace('href="/inspector.css"', 'href="inspector.css"').replace('src="/inspector.js"', 'src="inspector.js"')
html = html.replace('<script src="inspector.js" defer></script>', '<script src="desktop.js" defer></script><script src="inspector.js" defer></script><script src="apps.js" defer></script>')
(dist / "index.html").write_text(html)
for file in (APP / "ui").iterdir():
    if file.is_file():
        shutil.copy2(file, dist / file.name)
