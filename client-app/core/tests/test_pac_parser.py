"""Tests for client-app pac_parser.

Uses ``server-app/core/proxy.pac`` as the canonical fixture so the
parser stays in lock-step with whatever the server is actually serving.
"""
from __future__ import annotations

from pathlib import Path

from pac_parser import (
    _strip_js_comments,
    extract_patterns,
    extract_proxy_hosts,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
SERVER_PAC = REPO_ROOT / "server-app" / "core" / "proxy.pac"


def _read_real_pac() -> str:
    return SERVER_PAC.read_text(encoding="utf-8")


def test_strip_js_comments_removes_line_comments():
    src = "var a = 1;  // explanation\nvar b = 2;"
    assert "explanation" not in _strip_js_comments(src)
    assert "var a = 1" in _strip_js_comments(src)


def test_strip_js_comments_removes_block_comments():
    src = "x;\n/* dnsDomainIs(host, \"google.com\") */\ny;"
    cleaned = _strip_js_comments(src)
    assert "google.com" not in cleaned


def test_extract_proxy_hosts_finds_zoom_internal_section():
    out = extract_proxy_hosts(_read_real_pac())
    assert "*.zoom.us" in out
    assert "*.zoomdev.us" in out
    assert "*.eng.corp.zoom.com" in out
    assert "*.zoomvideo.atlassian.net" in out


def test_extract_proxy_hosts_finds_overseas_optional_section():
    out = extract_proxy_hosts(_read_real_pac())
    assert "*.google.com" in out
    assert "*.github.com" in out
    assert "*.openai.com" in out
    assert "*.anthropic.com" in out


def test_extract_proxy_hosts_does_not_include_cn_section():
    out = extract_proxy_hosts(_read_real_pac())
    assert "*.baidu.com" not in out
    assert "*.taobao.com" not in out
    assert "*.qq.com" not in out


def test_extract_proxy_hosts_does_not_include_local_section():
    out = extract_proxy_hosts(_read_real_pac())
    assert "*.local" not in out
    assert "*.lan" not in out
    assert "*.internal" not in out


def test_extract_patterns_returns_both_buckets():
    extraction = extract_patterns(_read_real_pac())
    assert extraction.proxy_patterns
    assert extraction.direct_patterns
    assert "*.baidu.com" in extraction.direct_patterns
    assert "*.zoom.us" in extraction.proxy_patterns


def test_dns_domain_normalised_with_wildcard_prefix():
    src = """
    function FindProxyForURL(url, host) {
        var PROXY = "PROXY x:1";
        if (dnsDomainIs(host, "example.com")) { return PROXY; }
        return "DIRECT";
    }
    """
    out = extract_proxy_hosts(src)
    assert out == ["*.example.com"]


def test_shexpmatch_preserved_verbatim():
    src = """
    function FindProxyForURL(url, host) {
        var PROXY = "PROXY x:1";
        if (shExpMatch(host, "git.zoom.us") || shExpMatch(host, "*.dev.example.com")) {
            return PROXY;
        }
        return "DIRECT";
    }
    """
    out = extract_proxy_hosts(src)
    assert "git.zoom.us" in out
    assert "*.dev.example.com" in out


def test_only_first_return_of_block_decides_bucket():
    src = """
    function FindProxyForURL(url, host) {
        if (dnsDomainIs(host, "x.com")) { return "PROXY foo:1"; }
        if (dnsDomainIs(host, "y.com")) { return "DIRECT"; }
        return "DIRECT";
    }
    """
    extraction = extract_patterns(src)
    assert "*.x.com" in extraction.proxy_patterns
    assert "*.y.com" in extraction.direct_patterns
    assert "*.x.com" not in extraction.direct_patterns


def test_commented_out_hosts_are_ignored():
    src = """
    function FindProxyForURL(url, host) {
        // dnsDomainIs(host, "secret.example.com")
        /* dnsDomainIs(host, "another.example.com") */
        if (dnsDomainIs(host, "real.example.com")) { return "PROXY x:1"; }
        return "DIRECT";
    }
    """
    out = extract_proxy_hosts(src)
    assert out == ["*.real.example.com"]


def test_empty_pac_returns_empty_lists():
    extraction = extract_patterns("// nothing here")
    assert extraction.proxy_patterns == []
    assert extraction.direct_patterns == []


def test_dedup_repeated_hosts():
    src = """
    function FindProxyForURL(url, host) {
        if (dnsDomainIs(host, "zoom.us")) { return "PROXY a:1"; }
        if (dnsDomainIs(host, "zoom.us")) { return "PROXY a:1"; }
        return "DIRECT";
    }
    """
    out = extract_proxy_hosts(src)
    assert out.count("*.zoom.us") == 1


def test_real_pac_has_expected_proxy_count_lower_bound():
    out = extract_proxy_hosts(_read_real_pac())
    # zoom internal (7) + overseas optional (~19) ~= 26
    assert len(out) >= 20
