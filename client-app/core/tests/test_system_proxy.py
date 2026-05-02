"""Tests for ``MacSystemProxy`` using a fake ``ProcessRunner``.

We never actually shell out to ``networksetup`` — the test injects a
recording fake that returns canned stdout for each command shape.

This keeps the test suite portable to Linux CI runners.

Cross-references:
* design/2026-04-30-3-Conduit-Client-客户端可行性报告.md §3.7
"""
from __future__ import annotations

from typing import Sequence

import pytest

from system_proxy import (
    DEFAULT_HOST,
    DEFAULT_PORT,
    MacSystemProxy,
    ProcessResult,
)


LISTALL_DEFAULT = (
    "An asterisk (*) denotes that a network service is disabled.\n"
    "Wi-Fi\n"
    "Ethernet\n"
    "*Bluetooth PAN\n"
    "Thunderbolt Bridge\n"
)

LISTALL_ETHERNET_ONLY = (
    "An asterisk (*) denotes that a network service is disabled.\n"
    "Ethernet\n"
)


# ---------------------------------------------------------------------------
# ProcessRunner fake
# ---------------------------------------------------------------------------


class FakeRunner:
    def __init__(self) -> None:
        self.calls: list[list[str]] = []
        self._scripted: list[ProcessResult] = []
        self._socks_state: dict[str, tuple[bool, str, int]] = {}
        self._listall_stdout = LISTALL_DEFAULT

    def set_listall(self, stdout: str) -> None:
        self._listall_stdout = stdout

    def set_socks(self, service: str, *, enabled: bool, server: str, port: int) -> None:
        self._socks_state[service] = (enabled, server, port)

    def script(self, result: ProcessResult) -> None:
        self._scripted.append(result)

    def __call__(self, args: Sequence[str]) -> ProcessResult:
        self.calls.append(list(args))
        if self._scripted:
            return self._scripted.pop(0)

        if "-listallnetworkservices" in args:
            return ProcessResult(list(args), 0, self._listall_stdout, "")

        if "-getsocksfirewallproxy" in args:
            svc = args[-1]
            enabled, server, port = self._socks_state.get(svc, (False, "", 0))
            stdout = (
                f"Enabled: {'Yes' if enabled else 'No'}\n"
                f"Server: {server}\n"
                f"Port: {port}\n"
                f"Authenticated Proxy Enabled: 0\n"
            )
            return ProcessResult(list(args), 0, stdout, "")

        if "-setsocksfirewallproxy" in args:
            svc = args[2]
            host = args[3]
            port = int(args[4])
            self._socks_state[svc] = (
                self._socks_state.get(svc, (False, "", 0))[0],
                host,
                port,
            )
            return ProcessResult(list(args), 0, "", "")

        if "-setsocksfirewallproxystate" in args:
            svc = args[2]
            new_state = args[3].lower() == "on"
            cur = self._socks_state.get(svc, (False, "", 0))
            self._socks_state[svc] = (new_state, cur[1], cur[2])
            return ProcessResult(list(args), 0, "", "")

        return ProcessResult(list(args), 1, "", "unknown command")


@pytest.fixture
def runner() -> FakeRunner:
    return FakeRunner()


@pytest.fixture
def proxy(runner: FakeRunner) -> MacSystemProxy:
    return MacSystemProxy(runner=runner)


# ---------------------------------------------------------------------------
# list_services
# ---------------------------------------------------------------------------


def test_list_services_strips_disabled_and_header(proxy: MacSystemProxy):
    assert proxy.list_services() == ["Wi-Fi", "Ethernet", "Thunderbolt Bridge"]


def test_active_service_prefers_wifi(proxy: MacSystemProxy):
    assert proxy.active_service() == "Wi-Fi"


def test_active_service_falls_back_to_ethernet_when_no_wifi(
    proxy: MacSystemProxy, runner: FakeRunner,
):
    runner.set_listall(LISTALL_ETHERNET_ONLY)
    assert proxy.active_service() == "Ethernet"


def test_list_services_raises_on_failure(runner: FakeRunner, proxy: MacSystemProxy):
    runner.script(ProcessResult([NETWORKSETUP_listall := "fail"], 1, "", "boom"))
    with pytest.raises(RuntimeError):
        proxy.list_services()


# ---------------------------------------------------------------------------
# enable / disable
# ---------------------------------------------------------------------------


def test_enable_runs_two_commands_and_records_state(
    proxy: MacSystemProxy, runner: FakeRunner,
):
    proxy.enable(host=DEFAULT_HOST, port=DEFAULT_PORT)
    cmd_set = [c for c in runner.calls if "-setsocksfirewallproxy" in c]
    cmd_state = [c for c in runner.calls if "-setsocksfirewallproxystate" in c]
    assert len(cmd_set) == 1
    assert len(cmd_state) == 1
    assert "Wi-Fi" in cmd_set[0]
    assert str(DEFAULT_PORT) in cmd_set[0]


def test_disable_calls_state_off(proxy: MacSystemProxy, runner: FakeRunner):
    proxy.enable()
    runner.calls.clear()
    proxy.disable()
    assert runner.calls[-1][:2] == ["/usr/sbin/networksetup", "-setsocksfirewallproxystate"]
    assert runner.calls[-1][-1] == "off"


def test_is_set_to_us_returns_true_when_pointed_at_us(
    proxy: MacSystemProxy, runner: FakeRunner,
):
    runner.set_socks("Wi-Fi", enabled=True, server=DEFAULT_HOST, port=DEFAULT_PORT)
    assert proxy.is_set_to_us() is True


def test_is_set_to_us_returns_false_when_not_pointed_at_us(
    proxy: MacSystemProxy, runner: FakeRunner,
):
    runner.set_socks("Wi-Fi", enabled=True, server="10.0.0.1", port=8080)
    assert proxy.is_set_to_us() is False


def test_is_set_to_us_returns_false_when_disabled(
    proxy: MacSystemProxy, runner: FakeRunner,
):
    runner.set_socks("Wi-Fi", enabled=False, server=DEFAULT_HOST, port=DEFAULT_PORT)
    assert proxy.is_set_to_us() is False


# ---------------------------------------------------------------------------
# cleanup_if_pointing_to_us
# ---------------------------------------------------------------------------


def test_cleanup_disables_only_services_pointing_to_us(
    proxy: MacSystemProxy, runner: FakeRunner,
):
    runner.set_socks("Wi-Fi", enabled=True, server=DEFAULT_HOST, port=DEFAULT_PORT)
    runner.set_socks("Ethernet", enabled=True, server="10.0.0.1", port=8080)
    runner.set_socks(
        "Thunderbolt Bridge", enabled=False, server=DEFAULT_HOST, port=DEFAULT_PORT,
    )

    cleaned = proxy.cleanup_if_pointing_to_us()
    assert cleaned is True

    state_calls = [c for c in runner.calls if "-setsocksfirewallproxystate" in c]
    services_disabled = [c[2] for c in state_calls if c[-1] == "off"]
    assert services_disabled == ["Wi-Fi"]


def test_cleanup_returns_false_when_nothing_to_do(
    proxy: MacSystemProxy, runner: FakeRunner,
):
    runner.set_socks("Wi-Fi", enabled=True, server="10.0.0.1", port=8080)
    runner.set_socks("Ethernet", enabled=False, server="", port=0)

    cleaned = proxy.cleanup_if_pointing_to_us()
    assert cleaned is False


# ---------------------------------------------------------------------------
# misc
# ---------------------------------------------------------------------------


def test_get_socks_proxy_parses_response(proxy: MacSystemProxy, runner: FakeRunner):
    runner.set_socks("Wi-Fi", enabled=True, server="127.0.0.1", port=7890)
    state = proxy.get_socks_proxy("Wi-Fi")
    assert state.enabled is True
    assert state.server == "127.0.0.1"
    assert state.port == 7890
    assert state.points_to("127.0.0.1", 7890) is True
    assert state.points_to("127.0.0.1", 8080) is False


def test_is_supported_uses_real_path_check(monkeypatch):
    """Sanity check — only verifies the function does *something* sensible."""
    import system_proxy
    monkeypatch.setattr(system_proxy.shutil, "which", lambda x: "/usr/sbin/networksetup")
    assert MacSystemProxy.is_supported() is True
    monkeypatch.setattr(system_proxy.shutil, "which", lambda x: None)
    assert MacSystemProxy.is_supported() is False
