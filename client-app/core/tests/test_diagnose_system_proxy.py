"""client-app diagnose._check_system_proxy 五个分支单元测试。

回归 2026-05-03 的修复:
  原行为: enable_system_proxy=True 但 _system_proxy_active=False 时 ok=False FAIL,
         误导用户以为连接坏了。
  新行为: 这种场景应当 ok=True + 友好 detail(把 networksetup 错误 brief 出来),
         remediation 提示手动配 SOCKS5。
"""
from __future__ import annotations

import builtins
import socket
from types import SimpleNamespace

import pytest

from api.diagnose import _check_system_proxy


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _runtime(
    *,
    sp=object(),
    enable=True,
    connected=True,
    active=False,
    last_err: str | None = None,
    actual_port: int = 18498,
):
    """构造最小 runtime stub 供 _check_system_proxy 使用。"""
    cfg = SimpleNamespace(enable_system_proxy=enable)
    proxy = SimpleNamespace(actual_port=actual_port)
    server = SimpleNamespace(server_id="x", host="1.2.3.4", port=24807,
                             socks=23326, api=19883) if connected else None
    return SimpleNamespace(
        cfg=cfg,
        proxy=proxy,
        connected_server=server,
        system_proxy=sp,
        _system_proxy_active=active,
        _system_proxy_last_error=last_err,
    )


# ---------- 分支 1: 平台不支持 ----------

def test_unsupported_platform_returns_ok_with_manual_hint():
    rt = _runtime(sp=None)
    out = _check_system_proxy(rt)
    assert out["ok"] is True
    assert out["key"] == "system_proxy"
    assert "非 macOS" in out["detail"] or "本平台" in out["detail"]


# ---------- 分支 2: 用户主动禁用 ----------

def test_disabled_by_user_returns_ok_no_remediation():
    rt = _runtime(enable=False)
    out = _check_system_proxy(rt)
    assert out["ok"] is True
    assert out["remediation"] is None
    # detail 提到禁用标志或环境变量
    assert "禁用" in out["detail"] or "no-system-proxy" in out["detail"]


# ---------- 分支 3: 未连接 ----------

def test_disconnected_returns_ok_says_state_preserved():
    rt = _runtime(connected=False)
    out = _check_system_proxy(rt)
    assert out["ok"] is True
    assert "未连接" in out["detail"]
    assert out["remediation"] is None


# ---------- 分支 4: 已连接但未激活(回归测试) ----------

def test_connected_but_inactive_no_error_returns_ok_with_manual_socks_hint():
    """回归: 之前会返回 ok=False FAIL,现在必须 ok=True。"""
    rt = _runtime(active=False, last_err=None, actual_port=18498)
    out = _check_system_proxy(rt)
    assert out["ok"] is True, "已连接但 system_proxy 未激活不应报 FAIL,因为代理仍然可用(用户手动配 SOCKS5 即可)"
    assert "127.0.0.1:18498" in out["detail"]
    # remediation 应给出手动配 SOCKS5 引导
    assert out["remediation"] is not None
    assert "SOCKS5" in out["remediation"]


def test_connected_but_inactive_with_networksetup_error_briefs_first_line():
    """networksetup 报错应当展示在 detail,但只截取首行避免过长。"""
    err = (
        "networksetup -setsocksfirewallproxy 'Wi-Fi' failed: "
        "** Error: Command requires admin privileges.  "
        "(macOS 13+ 要求管理员权限才能修改系统代理。)"
    )
    rt = _runtime(active=False, last_err=err, actual_port=18498)
    out = _check_system_proxy(rt)
    assert out["ok"] is True
    # 错误首行被附加到 detail
    assert "networksetup" in out["detail"]
    # remediation 解释 macOS 13+ 限制
    assert "macOS" in out["remediation"]
    assert "127.0.0.1" in out["remediation"]


def test_connected_but_inactive_truncates_overlong_error_line():
    """单行超长错误应当截断到 120 字符以内,detail 不至于撑爆 UI。"""
    err = "X" * 500  # 单行 500 字符
    rt = _runtime(active=False, last_err=err)
    out = _check_system_proxy(rt)
    assert out["ok"] is True
    # 截断到 120 后,加上前缀仍应远小于 500
    assert len(out["detail"]) < 250


def test_connected_but_inactive_multiline_error_takes_first_line_only():
    err = "first line of error\nstack trace blah blah\nmore noise"
    rt = _runtime(active=False, last_err=err)
    out = _check_system_proxy(rt)
    assert "first line of error" in out["detail"]
    assert "stack trace" not in out["detail"]


# ---------- 分支 5: 已连接且已激活 ----------

def test_connected_and_active_returns_ok_with_port():
    rt = _runtime(active=True, actual_port=18888)
    out = _check_system_proxy(rt)
    assert out["ok"] is True
    assert "18888" in out["detail"]
    assert out["remediation"] is None
