//! SDK HTTP transport. ABI references: PS5_PayloadSDK/samples/http2_get (net/SSL),
//! OpenOrbis include/orbis/Http.h and shadPS4 network/http.h (HTTP). TLS verification is
//! explicitly enabled. Redirects are handled here so HTTPS cannot silently become HTTP.
use super::{io, Fetch, Policy, Result};
use std::ffi::{c_char, c_int, c_void, CString};
use std::io::Write;
use std::time::{Duration, Instant};

// GitHub release redirects include CSP and signed-asset headers larger than the
// SDK default. Keep the native parser and our redirect parser at the same bound.
const RESPONSE_HEADER_LIMIT: usize = 64 * 1024;

extern "C" {
    fn sceNetInit() -> c_int;
    fn sceNetPoolCreate(name: *const c_char, size: c_int, flags: c_int) -> c_int;
    fn sceNetPoolDestroy(id: c_int) -> c_int;
    fn sceSslInit(size: usize) -> c_int;
    fn sceSslTerm(id: c_int) -> c_int;
    fn sceHttpInit(net: c_int, ssl: c_int, size: usize) -> c_int;
    fn sceHttpTerm(id: c_int) -> c_int;
    fn sceHttpCreateTemplate(ctx: c_int, agent: *const c_char, version: c_int, proxy: c_int) -> c_int;
    fn sceHttpDeleteTemplate(id: c_int) -> c_int;
    fn sceHttpCreateConnectionWithURL(template: c_int, url: *const c_char, keepalive: c_int) -> c_int;
    fn sceHttpDeleteConnection(id: c_int) -> c_int;
    fn sceHttpCreateRequestWithURL(connection: c_int, method: c_int, url: *const c_char, size: u64) -> c_int;
    fn sceHttpDeleteRequest(id: c_int) -> c_int;
    fn sceHttpSetAutoRedirect(id: c_int, enabled: c_int) -> c_int;
    fn sceHttpSetResponseHeaderMaxSize(id: c_int, size: usize) -> c_int;
    fn sceHttpSetResolveTimeOut(id: c_int, usec: u32) -> c_int;
    fn sceHttpSetConnectTimeOut(id: c_int, usec: u32) -> c_int;
    fn sceHttpSetSendTimeOut(id: c_int, usec: u32) -> c_int;
    fn sceHttpSetRecvTimeOut(id: c_int, usec: u32) -> c_int;
    fn sceHttpsEnableOption(id: c_int, flags: u32) -> c_int;
    fn sceHttpAddRequestHeader(id: c_int, name: *const c_char, value: *const c_char, mode: c_int) -> c_int;
    fn sceHttpSendRequest(id: c_int, data: *const c_void, size: usize) -> c_int;
    fn sceHttpGetStatusCode(id: c_int, status: *mut c_int) -> c_int;
    fn sceHttpGetAllResponseHeaders(id: c_int, headers: *mut *mut c_char, size: *mut usize) -> c_int;
    fn sceHttpReadData(id: c_int, buffer: *mut c_void, size: u32) -> c_int;
}
fn checked(name: &str, result: c_int) -> Result<c_int> {
    if result < 0 { Err(format!("{name}: 0x{:08x}", result as u32)) } else { Ok(result) }
}
pub struct Http { pool: c_int, ssl: c_int, http: c_int, template: c_int }
impl Http {
    pub fn new() -> Result<Self> {
        let mut h = Self { pool: -1, ssl: -1, http: -1, template: -1 };
        // Net may already be initialized by a loaded system module. Pool creation is the
        // authoritative usability check; this worker does not terminate shared libnet state.
        unsafe {
            let net = sceNetInit();
            crate::log::say!("update: sceNetInit=0x{:08x}", net as u32);
            h.pool = checked("sceNetPoolCreate", sceNetPoolCreate(c"AethusUpdate".as_ptr(), 1024 * 1024, 0))?;
            h.ssl = checked("sceSslInit", sceSslInit(2 * 1024 * 1024))?;
            h.http = checked("sceHttpInit", sceHttpInit(h.pool, h.ssl, 2 * 1024 * 1024))?;
            h.template = checked("sceHttpCreateTemplate", sceHttpCreateTemplate(h.http, c"AethusPS5Updater/1.0".as_ptr(), 2, 0))?;
            checked("sceHttpSetAutoRedirect", sceHttpSetAutoRedirect(h.template, 0))?;
            checked("sceHttpSetResponseHeaderMaxSize", sceHttpSetResponseHeaderMaxSize(h.template, RESPONSE_HEADER_LIMIT))?;
            checked("sceHttpSetResolveTimeOut", sceHttpSetResolveTimeOut(h.template, 5_000_000))?;
            checked("sceHttpSetConnectTimeOut", sceHttpSetConnectTimeOut(h.template, 10_000_000))?;
            checked("sceHttpSetSendTimeOut", sceHttpSetSendTimeOut(h.template, 10_000_000))?;
            checked("sceHttpSetRecvTimeOut", sceHttpSetRecvTimeOut(h.template, 15_000_000))?;
            // Server certificate, hostname, dates and trusted issuer. Never disable verification.
            checked("sceHttpsEnableOption", sceHttpsEnableOption(h.template, 0x01 | 0x04 | 0x08 | 0x10 | 0x20))?;
            for (name, value) in [(c"Accept-Encoding", c"identity"), (c"Cache-Control", c"no-cache")] {
                checked("sceHttpAddRequestHeader", sceHttpAddRequestHeader(h.template, name.as_ptr(), value.as_ptr(), 0))?;
            }
        }
        Ok(h)
    }
}
impl Drop for Http {
    fn drop(&mut self) { unsafe {
        if self.template >= 0 { sceHttpDeleteTemplate(self.template); }
        if self.http >= 0 { sceHttpTerm(self.http); }
        if self.ssl >= 0 { sceSslTerm(self.ssl); }
        if self.pool >= 0 { sceNetPoolDestroy(self.pool); }
    } }
}
struct Request { connection: c_int, request: c_int }
impl Drop for Request {
    fn drop(&mut self) { unsafe {
        if self.request >= 0 { sceHttpDeleteRequest(self.request); }
        if self.connection >= 0 { sceHttpDeleteConnection(self.connection); }
    } }
}
impl Fetch for Http {
    fn get(&mut self, initial: &str, policy: &Policy, out: &mut dyn Write, limit: u64) -> Result<()> {
        let mut url = initial.to_owned();
        let start = Instant::now();
        for hop in 0..=5 {
            policy.check_url(&url, hop != 0)?;
            let c_url = CString::new(url.as_str()).map_err(|e| e.to_string())?;
            let mut r = Request { connection: -1, request: -1 };
            let mut status = 0;
            unsafe {
                r.connection = checked("sceHttpCreateConnectionWithURL", sceHttpCreateConnectionWithURL(self.template, c_url.as_ptr(), 0))?;
                r.request = checked("sceHttpCreateRequestWithURL", sceHttpCreateRequestWithURL(r.connection, 0, c_url.as_ptr(), 0))?;
                checked("sceHttpSendRequest", sceHttpSendRequest(r.request, std::ptr::null(), 0))?;
                checked("sceHttpGetStatusCode", sceHttpGetStatusCode(r.request, &mut status))?;
            }
            crate::log::say!("update: HTTP status {status}, redirect hop {hop}");
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                let mut headers = std::ptr::null_mut(); let mut size = 0;
                unsafe { checked("sceHttpGetAllResponseHeaders", sceHttpGetAllResponseHeaders(r.request, &mut headers, &mut size))?; }
                if headers.is_null() || size > RESPONSE_HEADER_LIMIT { return Err("invalid redirect headers".into()); }
                let text = std::str::from_utf8(unsafe { std::slice::from_raw_parts(headers.cast::<u8>(), size) }).map_err(|e| e.to_string())?;
                let location = text.lines().filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("location")).map(|(_, v)| v.trim().trim_end_matches('\0'))
                    .ok_or("redirect has no Location")?;
                url = if location.starts_with('/') && !location.starts_with("//") {
                    let (scheme, authority) = super::url_parts(&url)?;
                    format!("{scheme}://{authority}{location}")
                } else { location.to_owned() };
                continue;
            }
            if status != 200 { return Err(format!("update HTTP status {status}")); }
            let mut bytes = 0u64;
            let mut buffer = [0u8; 128 * 1024];
            loop {
                if start.elapsed() > Duration::from_secs(600) { return Err("update transfer exceeded ten minutes".into()); }
                let count = unsafe { checked("sceHttpReadData", sceHttpReadData(r.request, buffer.as_mut_ptr().cast(), buffer.len() as u32))? } as usize;
                if count == 0 { break; }
                if count > buffer.len() { return Err("invalid HTTP read length".into()); }
                bytes = bytes.checked_add(count as u64).ok_or("download length overflow")?;
                if bytes > limit { return Err("download exceeds expected size".into()); }
                io(out.write_all(&buffer[..count]))?;
            }
            crate::log::say!("update: received {bytes} bytes");
            return Ok(());
        }
        Err("too many update redirects".into())
    }
}
