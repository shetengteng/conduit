"""PAC 规则解析 + 路由判定单元测试 —— pac_engine.py。

覆盖目标：
  - load_rules: 节区切分 / shExpMatch / dnsDomainIs / isInNet 解析
  - find_proxy: 5 个节区的优先级和 fallback 顺序
  - 边界场景: 空 host / plain host / IPv4 / 大小写
  - 容错: 文件不存在 / 节区缺失 -> 返回空 PacRules,find_proxy 仍可用
"""
from __future__ import annotations

from pathlib import Path

import pytest

from pac_engine import (
    Decision,
    PacRules,
    _domain_any,
    _glob_any,
    _ip_in_net,
    _is_plain_host,
    _looks_like_ipv4,
    _split_sections,
    find_proxy,
    load_rules,
)


# ---------- 工具函数 ----------

def test_is_plain_host_recognises_bare_names():
    assert _is_plain_host("intranet")
    assert _is_plain_host("dev-box")
    assert not _is_plain_host("example.com")
    assert not _is_plain_host("a.b")


def test_looks_like_ipv4_only_accepts_real_ipv4():
    assert _looks_like_ipv4("192.168.1.1")
    assert _looks_like_ipv4("10.0.0.0")
    assert not _looks_like_ipv4("example.com")
    assert not _looks_like_ipv4("10.0.0.300")


def test_ip_in_net_handles_cidr_correctly():
    assert _ip_in_net("192.168.1.5", "192.168.0.0", "255.255.0.0")
    assert _ip_in_net("10.1.2.3", "10.0.0.0", "255.0.0.0")
    assert not _ip_in_net("172.16.0.1", "192.168.0.0", "255.255.0.0")


def test_ip_in_net_invalid_input_returns_false():
    assert not _ip_in_net("not-an-ip", "192.168.0.0", "255.255.0.0")
    assert not _ip_in_net("192.168.1.1", "bogus", "255.255.0.0")


def test_glob_any_matches_first_pattern():
    assert _glob_any("foo.example.com", ["bar.*", "*.example.com"]) == "*.example.com"
    assert _glob_any("baz.com", ["*.example.com"]) is None


def test_domain_any_matches_dnsDomainIs_semantics():
    assert _domain_any("foo.example.com", ["example.com"]) == "example.com"
    assert _domain_any("example.com", ["example.com"]) == "example.com"
    assert _domain_any("notexample.com", ["example.com"]) is None
    assert _domain_any("example.com", [".example.com"]) == ".example.com"


# ---------- _split_sections ----------

def test_split_sections_extracts_each_block_in_order():
    text = """
        // some preamble
        // ---------- 1. local ----------
        rule_a
        // ---------- 2. internal ----------
        rule_b
        // ---------- 3. fallback ----------
        rule_c
    """
    sections = _split_sections(text)
    assert sorted(sections.keys()) == [1, 2, 3]
    assert "rule_a" in sections[1]
    assert "rule_b" in sections[2]
    assert "rule_c" in sections[3]


def test_split_sections_empty_returns_empty_dict():
    assert _split_sections("// nothing here") == {}


# ---------- load_rules ----------

@pytest.fixture
def sample_pac(tmp_path: Path) -> Path:
    p = tmp_path / "proxy.pac"
    p.write_text(
        '''
        // ---------- 1. local/private ----------
        if (shExpMatch(host, "*.local")) return "DIRECT";
        if (dnsDomainIs(host, "lan.example")) return "DIRECT";
        if (isInNet(host, "10.0.0.0", "255.0.0.0")) return "DIRECT";

        // ---------- 2. internal (must use VPN) ----------
        if (shExpMatch(host, "*.corp.example")) return "PROXY 1.2.3.4:8080";
        if (dnsDomainIs(host, "internal.example")) return "PROXY 1.2.3.4:8080";

        // ---------- 3. fallback (proxy w/o DIRECT) ----------
        if (shExpMatch(host, "*.cdn.example")) return "PROXY 1.2.3.4:8080";

        // ---------- 4. CN direct ----------
        if (shExpMatch(host, "*.cn")) return "DIRECT";
        if (dnsDomainIs(host, "baidu.com")) return "DIRECT";
        ''',
        encoding="utf-8",
    )
    return p


def test_load_rules_parses_all_section_helpers(sample_pac):
    rules = load_rules(sample_pac)
    assert rules.local_globs == ["*.local"]
    assert rules.local_domains == ["lan.example"]
    assert rules.local_nets == [("10.0.0.0", "255.0.0.0")]
    assert rules.internal_globs == ["*.corp.example"]
    assert rules.internal_domains == ["internal.example"]
    assert rules.fallback_globs == ["*.cdn.example"]
    assert rules.cn_direct_globs == ["*.cn"]
    assert rules.cn_direct_domains == ["baidu.com"]
    assert rules.source_path == str(sample_pac)


def test_load_rules_missing_file_returns_empty_rules(tmp_path: Path):
    rules = load_rules(tmp_path / "no-such-file.pac")
    assert rules.local_globs == []
    assert rules.internal_globs == []
    assert rules.proxy_target.startswith("PROXY ")


def test_update_proxy_target_updates_both_strings():
    rules = PacRules()
    rules.update_proxy_target("server.local", 9999)
    assert rules.proxy_target == "PROXY server.local:9999"
    assert rules.proxy_target_with_fallback == "PROXY server.local:9999; DIRECT"


# ---------- find_proxy 语义 ----------

@pytest.fixture
def loaded_rules(sample_pac) -> PacRules:
    rules = load_rules(sample_pac)
    rules.update_proxy_target("up", 7777)
    return rules


def test_find_proxy_empty_host_returns_default_direct(loaded_rules):
    d = find_proxy("", loaded_rules)
    assert d.proxy == "DIRECT"
    assert d.matched_section.startswith("5.")


def test_find_proxy_plain_host_takes_section_1(loaded_rules):
    d = find_proxy("intranet", loaded_rules)
    assert d.proxy == "DIRECT"
    assert "local" in d.matched_section


def test_find_proxy_localhost_special_case(loaded_rules):
    d = find_proxy("localhost", loaded_rules)
    assert d.proxy == "DIRECT"


def test_find_proxy_section1_glob_match(loaded_rules):
    d = find_proxy("foo.local", loaded_rules)
    assert d.proxy == "DIRECT"
    assert d.matched_pattern == "*.local"


def test_find_proxy_section1_dnsDomain_match(loaded_rules):
    d = find_proxy("box.lan.example", loaded_rules)
    assert d.proxy == "DIRECT"
    assert "dnsDomainIs(lan.example)" == d.matched_pattern


def test_find_proxy_section1_isInNet_match(loaded_rules):
    d = find_proxy("10.1.2.3", loaded_rules)
    assert d.proxy == "DIRECT"
    assert "isInNet" in d.matched_pattern


def test_find_proxy_section2_internal_uses_proxy_target(loaded_rules):
    d = find_proxy("svc.corp.example", loaded_rules)
    assert d.proxy == "PROXY up:7777"
    assert d.matched_section.startswith("2.")


def test_find_proxy_section3_fallback(loaded_rules):
    d = find_proxy("img.cdn.example", loaded_rules)
    assert d.proxy == "PROXY up:7777"
    assert d.matched_section.startswith("3.")


def test_find_proxy_section4_cn_direct(loaded_rules):
    d = find_proxy("anything.cn", loaded_rules)
    assert d.proxy == "DIRECT"
    assert d.matched_section.startswith("4.")


def test_find_proxy_default_falls_through(loaded_rules):
    d = find_proxy("unmatched.example.org", loaded_rules)
    assert d.proxy == "DIRECT"
    assert d.matched_section.startswith("5.")
    assert "no rule matched" in d.matched_pattern


def test_find_proxy_case_insensitive_host(loaded_rules):
    d = find_proxy("FOO.LOCAL", loaded_rules)
    assert d.proxy == "DIRECT"
    assert d.matched_pattern == "*.local"


def test_decision_to_dict_round_trip():
    d = Decision("PROXY 1.2.3.4:80", "section X", "pattern Y")
    out = d.to_dict()
    assert out == {
        "proxy": "PROXY 1.2.3.4:80",
        "matched_section": "section X",
        "matched_pattern": "pattern Y",
    }
