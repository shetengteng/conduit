"""单一版本号来源 —— 优先读 pyproject.toml,失败时退到 fallback string。

读取顺序:
1. 直接定位 pyproject.toml(开发模式;PyInstaller onedir 模式下若 build-sidecars.sh
   带了 ``--add-data pyproject.toml`` 也能命中)
2. 回退到 ``importlib.metadata``(如果当前包在虚拟环境里 pip install -e 过)
3. 最后退到 fallback —— 由 ``scripts/bump-version.sh`` 在 bump 后通过 sed 同步
   保证 fallback 也总是真实版本

为什么不光用 importlib.metadata:
- PyInstaller onedir 不会把 RECORD/dist-info 带进来,直接调会拿不到。
- ``pip install -e .`` 流程在打包流水线里也不一定走过。
直读文件最稳。
"""
from __future__ import annotations

import sys
from pathlib import Path

# !! 这行的字符串值由 scripts/bump-version.sh 自动同步,不要手动改。
_FALLBACK = "0.1.3"


def _read_from_pyproject() -> str | None:
    """从 _version.py 同目录上层找 pyproject.toml 并提取 version。"""
    candidates = [
        Path(__file__).resolve().parent / "pyproject.toml",
        # PyInstaller onedir: pyproject.toml 通过 --add-data 拷到二进制平级
        Path(sys.executable).resolve().parent / "pyproject.toml",
        # PyInstaller onedir 把 datas 放到 _internal/
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

        return _pkg_version("conduit-server-core")
    except Exception:  # noqa: BLE001
        return None


VERSION: str = _read_from_pyproject() or _read_from_metadata() or _FALLBACK
