//! `system_proxy_sc` —— macOS SystemConfiguration framework 直接调用,替代
//! `networksetup` CLI + `osascript` 提权方案。
//!
//! ## 为什么不用 networksetup
//!
//! `networksetup -setsocksfirewallproxy*` 在 macOS 13+ 普通用户下常 exit 14
//! ("Operation not permitted"),早期只能 fallback 到 `osascript ... with
//! administrator privileges`,但 osascript token **不跨进程缓存**,导致每次
//! connect 都弹密码框。
//!
//! ## 当前方案
//!
//! 走 SystemConfiguration framework + Authorization Services:
//!
//! 1. **进程级缓存 [`AuthRef`]**:首次调用 `enable` / `disable` 时通过
//!    `AuthorizationCreate(... extendRights | interactionAllowed | preAuthorize)`
//!    弹出 macOS 原生密码框(同步阻塞),用户授权后拿到的 `AuthorizationRef`
//!    缓存到全局 `OnceLock`。**之后整个进程内复用同一个 token,不再弹框**。
//!
//! 2. **SC 写**:`SCPreferencesCreateWithAuthorization(token)` 拿到带授权
//!    上下文的 Preferences ref → 遍历每个 NetworkService → 取 Proxies
//!    protocol 的 configuration dict → set
//!    [`kSCPropNetProxiesSOCKSEnable`] / [`kSCPropNetProxiesSOCKSProxy`] /
//!    [`kSCPropNetProxiesSOCKSPort`] → SetConfiguration → CommitChanges +
//!    ApplyChanges。**每次 enable/disable 都重新 create 一个 Preferences
//!    ref,函数返回时 RAII 自动 release**。早期版本曾把 ref 进程级缓存
//!    "create once, use forever",但实测多次 commit 后 configd 持有的
//!    generation 会过期,导致 `SCPreferencesCommitChanges` 反复失败、
//!    系统代理回滚到 disabled。重建 ref 跟 configd 多一次 80–200ms 握手,
//!    但 connect/disconnect 是低频路径,可接受。
//!
//! 3. **失败处理**:任何一步失败直接返 Err,**不再 fallback osascript**。
//!    上层 `core.rs::step_switch_endpoint` 会发 `system_proxy_warning` event
//!    让 UI 显示横幅,与 Python 版行为对齐。
//!
//! ## 局限
//!
//! - AuthorizationRef **不能跨进程持久化**(macOS 出于安全考虑禁止)。每次重
//!   启 client 会再弹一次密码框。这是 Apple 的硬性约束,要彻底消除只能装
//!   privileged helper(SMJobBless),需要 Apple Developer ID 签名,目前 ad-hoc
//!   签名做不到。
//! - 首次弹框必须在 main thread 之外,否则会卡 UI;调用方应该在 connect 工作
//!   线程里跑。本模块不主动切线程,由 caller 控制。
//!
//! ## 参考文档
//! - https://developer.apple.com/documentation/security/authorization-services
//! - https://developer.apple.com/documentation/systemconfiguration/scpreferencescreatewithauthorization

use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::ptr;
use std::sync::{Mutex, OnceLock};

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFMutableDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_foundation_sys::base::CFRelease;
use core_foundation_sys::dictionary::CFDictionaryRef;
use core_foundation_sys::propertylist::CFPropertyListRef;
use core_foundation_sys::string::CFStringRef;
// Authorization Services FFI 直接使用 security-framework-sys (替代手写 extern "C"),
// 但仍以 raw AuthorizationRef 喂给 SC API —— security-framework 的高层 wrapper
// 把 raw handle 设为 private 字段且没有 as_raw,这条路走不通。
use security_framework_sys::authorization as sf_auth;
use system_configuration_sys::network_configuration::{
    SCNetworkProtocolGetConfiguration, SCNetworkProtocolSetConfiguration, SCNetworkServiceCopyAll,
    SCNetworkServiceCopyProtocol, SCNetworkServiceGetName, SCNetworkServiceRef,
};
use system_configuration_sys::preferences::{
    AuthorizationRef, SCPreferencesApplyChanges, SCPreferencesCommitChanges,
    SCPreferencesCreateWithAuthorization, SCPreferencesRef,
};
use system_configuration_sys::schema_definitions::{
    kSCEntNetProxies, kSCPropNetProxiesSOCKSEnable, kSCPropNetProxiesSOCKSPort,
    kSCPropNetProxiesSOCKSProxy,
};

/// 公开版,供 `core.rs` 调(`core.rs` 里的状态机进出 connect_lock 的时机
/// 跟 SC API 调用一样关键,放在同一个 log 文件方便对照排查)。
pub fn diag_log_pub(msg: &str) {
    diag_log(msg);
}

/// 把诊断行 append 到 `~/Library/Logs/Conduit/conduit-client.log`,**用户重
/// 装/重启 client 不会清空**(env_logger 写 stderr,Tauri GUI 模式下 stderr
/// 默认丢弃,无法用 `log show` 看;此文件是用户态可见的唯一诊断渠道)。
fn diag_log(msg: &str) {
    let path = match diag_log_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or_default();
        let _ = writeln!(f, "[{ts:.3}] {msg}");
    }
}

fn diag_log_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push("Library");
    p.push("Logs");
    p.push("Conduit");
    p.push("conduit-client.log");
    Some(p)
}

// ---------------------------------------------------------------------------
// 进程级 AuthorizationRef 缓存
//
// 之前 (v0.2.2 及更早) 这里手写 extern "C" + 5 个 bit-flag 常量。v0.2.3
// 起改用 security-framework-sys::authorization 提供的 sys 函数与常量,
// 不再维护手写 FFI 声明。仍然走 raw AuthorizationRef (传给 SC API 必需),
// 因为 security-framework 高层 Authorization 把 handle 设为私有字段。

/// 持有 AuthorizationRef 的封装,Drop 时调 AuthorizationFree 释放 token。
/// 用 `unsafe impl Send/Sync` 因为 AuthorizationRef 是 macOS 提供的 thread-safe
/// 句柄(参考 Apple 文档),只是 Rust 的 raw pointer 类型默认非 Send/Sync。
struct AuthHolder {
    raw: AuthorizationRef,
}

unsafe impl Send for AuthHolder {}
unsafe impl Sync for AuthHolder {}

impl Drop for AuthHolder {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                // 不传 destroyRights,避免回收时把后续 SC 调用所需的授权也释放。
                sf_auth::AuthorizationFree(
                    self.raw as sf_auth::AuthorizationRef,
                    sf_auth::kAuthorizationFlagDefaults,
                );
            }
        }
    }
}

/// 全局 token 缓存。OnceLock 保证首次 init 是单线程的;Mutex 保护后续读取。
/// 一旦设值后基本不变,但用 Mutex 是为了在极端情况下(token 失效)允许重置。
static AUTH_CACHE: OnceLock<Mutex<Option<AuthHolder>>> = OnceLock::new();

/// 获取或创建进程级 AuthorizationRef。**第一次调用会同步阻塞弹出 macOS
/// 密码框**,用户输入密码并通过后返回有效 token;之后调用复用已缓存的 token,
/// 不再弹框。
///
/// 用户在密码框里点取消 / 输错密码 → 返回 `Err`,本次操作失败但不缓存,下次
/// 调用还会再弹。
fn get_or_create_auth() -> Result<AuthorizationRef, String> {
    let cell = AUTH_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().map_err(|e| format!("auth cache poisoned: {e}"))?;
    if let Some(holder) = guard.as_ref() {
        if !holder.raw.is_null() {
            return Ok(holder.raw);
        }
    }

    // 三个 flag 一起用是经典组合:extendRights 让 token 可用于 SC API,
    // interactionAllowed 允许必要时弹密码框,preAuthorize 让 OS 在创建时
    // 就完成授权(避免 SCPreferencesCreateWithAuthorization 内部再次触发弹框)。
    let flags = sf_auth::kAuthorizationFlagExtendRights
        | sf_auth::kAuthorizationFlagInteractionAllowed
        | sf_auth::kAuthorizationFlagPreAuthorize;
    let mut sf_handle: sf_auth::AuthorizationRef = ptr::null_mut();
    let status = unsafe {
        sf_auth::AuthorizationCreate(ptr::null(), ptr::null(), flags, &mut sf_handle as *mut _)
    };
    if status != sf_auth::errAuthorizationSuccess || sf_handle.is_null() {
        return Err(format!(
            "AuthorizationCreate failed: OSStatus={status} (user cancel = {}, denied = {})",
            sf_auth::errAuthorizationCanceled, sf_auth::errAuthorizationDenied
        ));
    }
    // SC API 用的 AuthorizationRef 是 *const c_void,sf_auth 用的是 *mut c_void;
    // 同一个 token 跨 sys crate,只是常量类型签名不同,做个 cast 即可。
    let auth = sf_handle as AuthorizationRef;

    let holder = AuthHolder { raw: auth };
    let raw = holder.raw;
    *guard = Some(holder);
    Ok(raw)
}

/// 强制重置缓存(token 失效或测试需要)。当前不暴露,留给后续调试。
#[allow(dead_code)]
fn reset_auth() {
    if let Some(cell) = AUTH_CACHE.get() {
        if let Ok(mut guard) = cell.lock() {
            // 让旧的 AuthHolder 走 Drop → AuthorizationFree
            *guard = None;
        }
    }
}

// ---------------------------------------------------------------------------
// SCPreferences helper

/// 包装 SCPreferencesRef 的 RAII handle,Drop 时 CFRelease。
///
/// 之前的实现把 `SCPreferencesRef` 进程级缓存"create once, use forever",
/// 但实测 disconnect→reconnect 几次后 `SCPreferencesCommitChanges` 会反复
/// 失败(详见 `~/Library/Logs/Conduit/conduit-client.log`),原因是 commit
/// 之后 ref 内部持有的 generation token 已过期,configd 拒绝再次 commit。
/// 改为**每次 enable/disable 都新建 ref,函数返回时 RAII 自动 release**,
/// 即 Apple 文档对 SCPreferences 的标准用法。开销是每次操作多一次跟 configd
/// 的握手(80-200ms),connect/disconnect 是低频操作,完全可接受。
struct PrefsHolder {
    raw: SCPreferencesRef,
}

unsafe impl Send for PrefsHolder {}
unsafe impl Sync for PrefsHolder {}

impl Drop for PrefsHolder {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                CFRelease(self.raw as _);
            }
        }
    }
}

/// 新建一个 `SCPreferencesRef`(带授权)并用 RAII holder 包起来,函数返回
/// 时 holder Drop 自动 `CFRelease`。**不**做进程级缓存。
///
/// `auth` 仍由 [`get_or_create_auth`] 提供(Authorization token 跨多次
/// SC 操作复用是安全的,Apple 文档允许;真正不该跨多次操作复用的是
/// `SCPreferencesRef` 本身)。
fn create_prefs(auth: AuthorizationRef) -> Result<PrefsHolder, String> {
    let name = CFString::new("com.terrellshe.conduit.client.system_proxy");
    let raw = unsafe {
        SCPreferencesCreateWithAuthorization(
            ptr::null(),
            name.as_concrete_TypeRef(),
            ptr::null(),
            auth,
        )
    };
    if raw.is_null() {
        return Err("SCPreferencesCreateWithAuthorization returned NULL".into());
    }
    Ok(PrefsHolder { raw })
}

// ---------------------------------------------------------------------------
// 公共 API

/// 把所有目标 NetworkService 的 SOCKS proxy 改成 `host:port` 并启用。
///
/// `target_service_names` 是 [`super::system_proxy::pick_target_services`] 过
/// 滤后的 service 名列表(英文,与 networksetup -listallnetworkservices 输出
/// 一致)。本函数只对名字匹配的 service 写,其它跳过。
///
/// 返回成功更新的 service 数量;0 通常表示 services 名都没匹配上,调用方应警告。
pub fn enable_via_sc(
    target_service_names: &[String],
    host: &str,
    port: u16,
) -> Result<usize, String> {
    if target_service_names.is_empty() {
        return Err("enable_via_sc: empty target service list".into());
    }
    diag_log(&format!(
        "enable_via_sc START host={host} port={port} services={target_service_names:?}"
    ));
    let t0 = std::time::Instant::now();
    let auth = get_or_create_auth()?;
    let t_auth = t0.elapsed();
    diag_log(&format!("enable_via_sc auth_done in {t_auth:?}"));
    let prefs_holder = create_prefs(auth)?;
    let prefs = prefs_holder.raw;
    let t_create = t0.elapsed();
    diag_log(&format!(
        "enable_via_sc prefs_done in {:?} (cumul {t_create:?})",
        t_create - t_auth
    ));
    let updated = mutate_proxy_for_services(prefs, target_service_names, |dict| {
        write_socks_into_dict(dict, host, port, true);
    })?;
    let t_mutate = t0.elapsed();
    diag_log(&format!(
        "enable_via_sc mutate_done updated={updated} in {:?} (cumul {t_mutate:?})",
        t_mutate - t_create
    ));
    commit_and_apply(prefs)?;
    let t_commit = t0.elapsed();
    diag_log(&format!(
        "enable_via_sc commit_done in {:?} (cumul {t_commit:?})",
        t_commit - t_mutate
    ));
    log::info!(
        "[system_proxy_sc] enable {host}:{port} updated={updated} services in {:?} \
         (auth={:?} prefs={:?} mutate={:?} commit={:?})",
        t_commit,
        t_auth,
        t_create - t_auth,
        t_mutate - t_create,
        t_commit - t_mutate,
    );
    diag_log(&format!("enable_via_sc OK total={t_commit:?}"));
    Ok(updated)
}

/// 把所有目标 NetworkService 的 SOCKS proxy 关掉(SOCKSEnable=0)。
/// 不清除 host/port 字段,这样下次 enable 不需要重新输入。
pub fn disable_via_sc(target_service_names: &[String]) -> Result<usize, String> {
    if target_service_names.is_empty() {
        return Ok(0);
    }
    diag_log(&format!(
        "disable_via_sc START services={target_service_names:?}"
    ));
    let t0 = std::time::Instant::now();
    let auth = get_or_create_auth()?;
    let t_auth = t0.elapsed();
    diag_log(&format!("disable_via_sc auth_done in {t_auth:?}"));
    let prefs_holder = create_prefs(auth)?;
    let prefs = prefs_holder.raw;
    let t_create = t0.elapsed();
    diag_log(&format!(
        "disable_via_sc prefs_done in {:?} (cumul {t_create:?})",
        t_create - t_auth
    ));
    let updated = mutate_proxy_for_services(prefs, target_service_names, |dict| {
        let zero = CFNumber::from(0_i32);
        dict.set(
            unsafe { CFString::wrap_under_get_rule(kSCPropNetProxiesSOCKSEnable) },
            zero.as_CFType(),
        );
    })?;
    let t_mutate = t0.elapsed();
    diag_log(&format!(
        "disable_via_sc mutate_done updated={updated} in {:?} (cumul {t_mutate:?})",
        t_mutate - t_create
    ));
    commit_and_apply(prefs)?;
    let t_commit = t0.elapsed();
    diag_log(&format!(
        "disable_via_sc commit_done in {:?} (cumul {t_commit:?})",
        t_commit - t_mutate
    ));
    log::info!(
        "[system_proxy_sc] disable updated={updated} services in {:?} \
         (auth={:?} prefs={:?} mutate={:?} commit={:?})",
        t_commit,
        t_auth,
        t_create - t_auth,
        t_mutate - t_create,
        t_commit - t_mutate,
    );
    diag_log(&format!("disable_via_sc OK total={t_commit:?}"));
    Ok(updated)
}

// ---------------------------------------------------------------------------
// 内部逻辑

/// 遍历 prefs 下所有 NetworkService,对名字命中 `target_names` 的
/// service,取出 Proxies protocol 的 configuration dict,clone 一份成
/// mutable,跑 `mutate(&mut dict)`,然后 SetConfiguration 回去。
///
/// 返回成功更新的 service 数量。
fn mutate_proxy_for_services(
    prefs: SCPreferencesRef,
    target_names: &[String],
    mut mutate: impl FnMut(&mut CFMutableDictionary<CFString, CFType>),
) -> Result<usize, String> {
    let services_arr_raw = unsafe { SCNetworkServiceCopyAll(prefs) };
    if services_arr_raw.is_null() {
        return Err("SCNetworkServiceCopyAll returned NULL".into());
    }
    let services: CFArray<SCNetworkServiceRef> =
        unsafe { CFArray::wrap_under_create_rule(services_arr_raw) };

    let mut updated = 0usize;
    for i in 0..services.len() {
        let svc_ref = match services.get(i) {
            Some(r) => *r,
            None => continue,
        };
        let name_ref = unsafe { SCNetworkServiceGetName(svc_ref) };
        if name_ref.is_null() {
            continue;
        }
        let name_cf = unsafe { CFString::wrap_under_get_rule(name_ref) };
        let name = name_cf.to_string();
        if !target_names.iter().any(|n| n == &name) {
            continue;
        }

        let proto_ref = unsafe {
            SCNetworkServiceCopyProtocol(svc_ref, kSCEntNetProxies)
        };
        if proto_ref.is_null() {
            log::warn!("[system_proxy_sc] no proxies protocol on service '{name}'");
            continue;
        }
        // SCNetworkServiceCopyProtocol returns +1 retain; we own it.
        let result = (|| -> Result<(), String> {
            let cfg_ref = unsafe { SCNetworkProtocolGetConfiguration(proto_ref) };
            // Get rule: read-only borrow,我们要 mutable copy。
            let mut new_dict: CFMutableDictionary<CFString, CFType> = if cfg_ref.is_null() {
                CFMutableDictionary::with_capacity(8)
            } else {
                copy_dict_to_mutable(cfg_ref)
            };
            mutate(&mut new_dict);
            let ok = unsafe {
                SCNetworkProtocolSetConfiguration(
                    proto_ref,
                    new_dict.as_concrete_TypeRef() as CFDictionaryRef,
                )
            };
            if ok == 0 {
                return Err(format!(
                    "SCNetworkProtocolSetConfiguration failed on service '{name}'"
                ));
            }
            updated += 1;
            log::info!("[system_proxy_sc] updated SOCKS proxy on service '{name}'");
            Ok(())
        })();
        unsafe { CFRelease(proto_ref as _) };
        result?;
    }

    Ok(updated)
}

/// 把 (kSCPropNetProxiesSOCKSEnable=1, SOCKSProxy=host, SOCKSPort=port)
/// 写进 dict。复用 dict 里其它字段(HTTP proxy / PAC 等)不动。
fn write_socks_into_dict(
    dict: &mut CFMutableDictionary<CFString, CFType>,
    host: &str,
    port: u16,
    enable: bool,
) {
    unsafe {
        let key_enable = CFString::wrap_under_get_rule(kSCPropNetProxiesSOCKSEnable);
        let key_proxy = CFString::wrap_under_get_rule(kSCPropNetProxiesSOCKSProxy);
        let key_port = CFString::wrap_under_get_rule(kSCPropNetProxiesSOCKSPort);

        let v_enable = CFNumber::from(if enable { 1_i32 } else { 0_i32 });
        let v_host = CFString::new(host);
        let v_port = CFNumber::from(port as i32);

        dict.set(key_enable, v_enable.as_CFType());
        dict.set(key_proxy, v_host.as_CFType());
        dict.set(key_port, v_port.as_CFType());
    }
    let _ = CFBoolean::true_value();
}

/// 把不可变 CFDictionaryRef 拷贝成 CFMutableDictionary<CFString, CFType>。
/// 简化处理:取 CFType 通用值,反正我们只 set 几个固定 key,其它原样保留。
fn copy_dict_to_mutable(src: CFDictionaryRef) -> CFMutableDictionary<CFString, CFType> {
    unsafe {
        let count = core_foundation_sys::dictionary::CFDictionaryGetCount(src) as usize;
        let mut keys: Vec<*const c_void> = vec![ptr::null(); count];
        let mut values: Vec<*const c_void> = vec![ptr::null(); count];
        core_foundation_sys::dictionary::CFDictionaryGetKeysAndValues(
            src,
            keys.as_mut_ptr() as *mut _,
            values.as_mut_ptr() as *mut _,
        );
        let mut out: CFMutableDictionary<CFString, CFType> =
            CFMutableDictionary::with_capacity(count as isize);
        for i in 0..count {
            let k_ref = keys[i] as CFStringRef;
            let v_ref = values[i] as CFPropertyListRef;
            if k_ref.is_null() || v_ref.is_null() {
                continue;
            }
            let k = CFString::wrap_under_get_rule(k_ref);
            let v = CFType::wrap_under_get_rule(v_ref as CFTypeRef);
            out.set(k, v);
        }
        out
    }
}

/// SCPreferencesCommitChanges + SCPreferencesApplyChanges。两步缺一不可:
/// commit 写到 plist 文件,apply 通知 configd 让运行中的系统看到变化。
fn commit_and_apply(prefs: SCPreferencesRef) -> Result<(), String> {
    unsafe {
        if SCPreferencesCommitChanges(prefs) == 0 {
            return Err("SCPreferencesCommitChanges failed".into());
        }
        if SCPreferencesApplyChanges(prefs) == 0 {
            return Err("SCPreferencesApplyChanges failed".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只测无网络副作用的 helper。SC API + AuthorizationCreate 在测试环境
    /// 会真的调用 macOS 系统服务,无法 mock,留给手工验证。
    /// 这里 sanity-check 一下 sys crate 提供的常量与 Apple 文档值一致。
    #[test]
    fn flag_constants_match_apple_documented_values() {
        assert_eq!(sf_auth::kAuthorizationFlagDefaults, 0);
        assert_eq!(sf_auth::kAuthorizationFlagInteractionAllowed, 1);
        assert_eq!(sf_auth::kAuthorizationFlagExtendRights, 2);
        assert_eq!(sf_auth::kAuthorizationFlagPreAuthorize, 16);
        assert_eq!(sf_auth::kAuthorizationFlagDestroyRights, 8);
        assert_eq!(sf_auth::errAuthorizationSuccess, 0);
    }

    #[test]
    fn auth_holder_drop_does_not_panic_on_null() {
        let h = AuthHolder { raw: ptr::null() };
        drop(h); // 不应该 panic
    }
}
