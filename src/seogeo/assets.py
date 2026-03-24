from __future__ import annotations

"""Image and asset inspection helpers."""

from dataclasses import dataclass
from pathlib import Path
import struct
import xml.etree.ElementTree as ET


@dataclass(slots=True)
class AssetInfo:
    """Normalized metadata for an image-like asset."""

    path: Path
    exists: bool
    byte_size: int | None = None
    width: int | None = None
    height: int | None = None

    @property
    def aspect_ratio(self) -> float | None:
        """Return width/height when both dimensions are known and valid."""
        if not self.width or not self.height:
            return None
        return self.width / self.height


def read_png_dimensions(path: Path) -> tuple[int, int] | None:
    """Read PNG width and height from the IHDR header."""
    data = path.read_bytes()[:24]
    if len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    return struct.unpack(">II", data[16:24])


def read_jpeg_dimensions(path: Path) -> tuple[int, int] | None:
    """Read JPEG dimensions from the first SOF marker."""
    data = path.read_bytes()
    if len(data) < 4 or data[:2] != b"\xff\xd8":
        return None
    index = 2
    while index + 9 < len(data):
        if data[index] != 0xFF:
            index += 1
            continue
        marker = data[index + 1]
        index += 2
        if marker in {0xD8, 0xD9}:
            continue
        length = struct.unpack(">H", data[index : index + 2])[0]
        if marker in {0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF}:
            height, width = struct.unpack(">HH", data[index + 3 : index + 7])
            return width, height
        index += length
    return None


def read_svg_dimensions(path: Path) -> tuple[int, int] | None:
    """Read SVG dimensions from width/height or viewBox."""
    try:
        root = ET.fromstring(path.read_text())
    except Exception:
        return None
    width_value = root.attrib.get("width")
    height_value = root.attrib.get("height")
    if width_value and height_value:
        try:
            return int(float(width_value.rstrip("px"))), int(float(height_value.rstrip("px")))
        except ValueError:
            pass
    view_box = root.attrib.get("viewBox")
    if not view_box:
        return None
    parts = view_box.replace(",", " ").split()
    if len(parts) != 4:
        return None
    try:
        return int(float(parts[2])), int(float(parts[3]))
    except ValueError:
        return None


def inspect_image_asset(path: Path) -> AssetInfo:
    """Inspect an image asset and return size plus dimensions when available."""
    if not path.exists():
        return AssetInfo(path=path, exists=False)
    width = height = None
    suffix = path.suffix.lower()
    try:
        if suffix == ".png":
            size = read_png_dimensions(path)
        elif suffix in {".jpg", ".jpeg"}:
            size = read_jpeg_dimensions(path)
        elif suffix == ".svg":
            size = read_svg_dimensions(path)
        else:
            size = None
    except Exception:
        size = None
    if size is not None:
        width, height = size
    return AssetInfo(
        path=path,
        exists=True,
        byte_size=path.stat().st_size,
        width=width,
        height=height,
    )
