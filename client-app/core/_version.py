"""单一版本号来源 —— 优先读 pyproject.toml,失败时退到 fallback string。

读取顺序:
1. 直接定位 pyproject.toml(开发模式;PyInstaller onedir 模式下若 build-sidecars.sh
   带了 ``--add-data pyproject.toml`` 也能命中)
2. 回退到 ``importlib.metadata``(如果当前包在虚拟环境里 pip install -e 过)
3. 最后退到 fallback —— 由 ``scripts/bump-version.sh`` 在 bump 后通过 sed 同步
   保证 fallback 也总是真实版本

详细说明见 server-app/core/_version.py。
"""
from __future__ import annotations

import sys
from pathlib import Path

# !! 这行的字符串值由 scripts/bump-version.sh 自动同步,不要手动改。
_FALLBACK = "0.1.2"


def _read_from_pyproject() -> str | None:
    candidates = [
        Path(__file__).resolve().parent / "pyproject.toml",
        Path(sys.executable).resolve().parent / "pyproject.toml",
        Path(sys.executable).resolve().parent / "_internal" / "pyproject.toml",
    ]
    for path in candidates:
        if not path.is_file():
            continue
        try:
            import tomllib  # type: ignore[import-not-found]

            data = tomllib.loads(path.read_text(encoding="utf-8"))
            v = data.get("project", {}).get("version")
            if isinstance(v, str) and v:
                return v
        except Exception:  # noqa: BLE001
            continue
    return None


def _read_from_metadata() -> str | None:
    try:
        from importlib.metadata import version as _pkg_version

        return _pkg_version("conduit-client-core")
    except Exception:  # noqa: BLE001
        return None


VERSION: str = _read_from_pyproject() or _read_from_metadata() or _FALLBACK
