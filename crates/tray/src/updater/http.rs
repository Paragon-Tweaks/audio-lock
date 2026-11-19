use std::ffi::c_void;
use std::fs::{self, File};
use std::io::Write;
use std::mem::size_of;
use std::path::Path;
use std::ptr;

use audio_lock_core::update::CURRENT_VERSION;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::Networking::WinHttp::{
    URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_INTERNET_SCHEME_HTTP, WINHTTP_INTERNET_SCHEME_HTTPS, WINHTTP_OPEN_REQUEST_FLAGS,
    WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_ALWAYS,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpCrackUrl, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
};
use windows::core::{PCWSTR, w};

use crate::win32;

const RESOLVE_TIMEOUT_MS: i32 = 10_000;
const CONNECT_TIMEOUT_MS: i32 = 10_000;
const SEND_TIMEOUT_MS: i32 = 15_000;
const RECEIVE_TIMEOUT_MS: i32 = 30_000;
const CHUNK_SIZE: usize = 16 * 1024;

struct WinHttpHandle(*mut c_void);

impl WinHttpHandle {
    fn new(raw: *mut c_void, context: &str) -> Result<Self, String> {
        if raw.is_null() {
            let err = unsafe { GetLastError() };
            return Err(format!("{context} failed (win32 error {})", err.0));
        }
        Ok(Self(raw))
    }

    fn raw(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for WinHttpHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

struct ParsedUrl {
    host: Vec<u16>,
    object_name: Vec<u16>,
    port: u16,
    is_https: bool,
}

fn parse_url(url: &str) -> Result<ParsedUrl, String> {
    let url_utf16: Vec<u16> = url.encode_utf16().collect();
    if url_utf16.is_empty() {
        return Err("Empty URL".to_string());
    }

    let mut components = URL_COMPONENTS {
        dwStructSize: size_of::<URL_COMPONENTS>() as u32,
        dwHostNameLength: u32::MAX,
        dwUrlPathLength: u32::MAX,
        dwExtraInfoLength: u32::MAX,
        ..Default::default()
    };

    unsafe { WinHttpCrackUrl(&url_utf16, 0, &mut components) }
        .map_err(|err| format!("Invalid URL: {err}"))?;

    let is_https = match components.nScheme {
        s if s == WINHTTP_INTERNET_SCHEME_HTTPS => true,
        s if s == WINHTTP_INTERNET_SCHEME_HTTP => false,
        _ => return Err("Unsupported URL scheme".to_string()),
    };

    if components.lpszHostName.is_null() || components.dwHostNameLength == 0 {
        return Err("URL missing host".to_string());
    }

    let host_slice = unsafe {
        std::slice::from_raw_parts(
            components.lpszHostName.0,
            components.dwHostNameLength as usize,
        )
    };
    let mut host = Vec::with_capacity(host_slice.len() + 1);
    host.extend_from_slice(host_slice);
    host.push(0);

    let path_slice = if !components.lpszUrlPath.is_null() && components.dwUrlPathLength > 0 {
        unsafe {
            std::slice::from_raw_parts(
                components.lpszUrlPath.0,
                components.dwUrlPathLength as usize,
            )
        }
    } else {
        &[]
    };
    let extra_slice = if !components.lpszExtraInfo.is_null() && components.dwExtraInfoLength > 0 {
        unsafe {
            std::slice::from_raw_parts(
                components.lpszExtraInfo.0,
                components.dwExtraInfoLength as usize,
            )
        }
    } else {
        &[]
    };

    let mut object_name = Vec::with_capacity(path_slice.len() + extra_slice.len() + 2);
    if path_slice.is_empty() {
        object_name.push(b'/' as u16);
    } else {
        object_name.extend_from_slice(path_slice);
    }
    object_name.extend_from_slice(extra_slice);
    object_name.push(0);

    Ok(ParsedUrl {
        host,
        object_name,
        port: components.nPort,
        is_https,
    })
}

struct OpenRequest {
    request: WinHttpHandle,
    _connect: WinHttpHandle,
    _session: WinHttpHandle,
}

fn open_get_request(url: &str) -> Result<OpenRequest, String> {
    let parsed = parse_url(url)?;
    let user_agent = win32::to_wide(&format!("default-audio-lock/{CURRENT_VERSION}"));

    let session = WinHttpHandle::new(
        unsafe {
            WinHttpOpen(
                PCWSTR(user_agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            )
        },
        "WinHttpOpen",
    )?;

    unsafe {
        WinHttpSetTimeouts(
            session.raw(),
            RESOLVE_TIMEOUT_MS,
            CONNECT_TIMEOUT_MS,
            SEND_TIMEOUT_MS,
            RECEIVE_TIMEOUT_MS,
        )
    }
    .map_err(|err| format!("WinHttpSetTimeouts failed: {err}"))?;

    let connect = WinHttpHandle::new(
        unsafe { WinHttpConnect(session.raw(), PCWSTR(parsed.host.as_ptr()), parsed.port, 0) },
        "WinHttpConnect",
    )?;

    let flags = if parsed.is_https {
        WINHTTP_FLAG_SECURE
    } else {
        WINHTTP_OPEN_REQUEST_FLAGS(0)
    };
    let request = WinHttpHandle::new(
        unsafe {
            WinHttpOpenRequest(
                connect.raw(),
                w!("GET"),
                PCWSTR(parsed.object_name.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                ptr::null(),
                flags,
            )
        },
        "WinHttpOpenRequest",
    )?;

    let redirect_policy = WINHTTP_OPTION_REDIRECT_POLICY_ALWAYS;
    unsafe {
        WinHttpSetOption(
            Some(request.raw()),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&redirect_policy.to_ne_bytes()),
        )
    }
    .map_err(|err| format!("WinHttpSetOption(redirect policy) failed: {err}"))?;

    unsafe { WinHttpSendRequest(request.raw(), None, None, 0, 0, 0) }
        .map_err(|err| format!("WinHttpSendRequest failed: {err}"))?;

    unsafe { WinHttpReceiveResponse(request.raw(), ptr::null_mut()) }
        .map_err(|err| format!("WinHttpReceiveResponse failed: {err}"))?;

    let mut status_code = 0u32;
    let mut status_len = size_of::<u32>() as u32;
    unsafe {
        WinHttpQueryHeaders(
            request.raw(),
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut status_code).cast::<c_void>()),
            &mut status_len,
            ptr::null_mut(),
        )
    }
    .map_err(|err| format!("WinHttpQueryHeaders failed: {err}"))?;

    if status_code != 200 {
        return Err(format!("Unexpected HTTP status {status_code}"));
    }

    Ok(OpenRequest {
        request,
        _connect: connect,
        _session: session,
    })
}

fn stream_response<F>(req: &OpenRequest, max_bytes: usize, mut on_chunk: F) -> Result<(), String>
where
    F: FnMut(&[u8]) -> Result<(), String>,
{
    let mut buf = [0u8; CHUNK_SIZE];
    let mut total = 0usize;

    loop {
        let mut bytes_read = 0u32;
        unsafe {
            WinHttpReadData(
                req.request.raw(),
                buf.as_mut_ptr().cast::<c_void>(),
                buf.len() as u32,
                &mut bytes_read,
            )
        }
        .map_err(|err| format!("WinHttpReadData failed: {err}"))?;

        if bytes_read == 0 {
            break;
        }

        let n = bytes_read as usize;
        total = total.saturating_add(n);
        if total > max_bytes {
            return Err(format!("Response exceeded limit of {max_bytes} bytes"));
        }

        on_chunk(&buf[..n])?;
    }

    Ok(())
}

pub fn fetch_bytes(url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    let req = open_get_request(url)?;
    let mut out = Vec::new();
    stream_response(&req, max_bytes, |chunk| {
        out.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(out)
}

pub fn download_to_file(url: &str, dest: &Path, max_bytes: usize) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("Failed to create download dir: {err}"))?;
    }

    let req = open_get_request(url)?;
    let mut file =
        File::create(dest).map_err(|err| format!("Failed to create download file: {err}"))?;

    let result = stream_response(&req, max_bytes, |chunk| {
        file.write_all(chunk)
            .map_err(|err| format!("Failed to write download chunk: {err}"))
    })
    .and_then(|()| {
        file.flush()
            .map_err(|err| format!("Failed to flush download file: {err}"))
    });

    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(dest);
    }

    result
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    fn spawn_http_server(responses: Vec<&'static str>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local listener");
        let addr = listener.local_addr().expect("local addr");
        thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut req_buf = [0u8; 2048];
                let _ = stream.read(&mut req_buf);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn fetches_bytes_and_follows_redirect() {
        let base = spawn_http_server(vec![
            "HTTP/1.1 302 Found\r\nLocation: /final?v=1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello world",
        ]);
        let bytes = fetch_bytes(&format!("{base}/start"), 1024).expect("fetch should succeed");
        assert_eq!(bytes, b"hello world");
    }

    #[test]
    fn enforces_max_bytes_limit() {
        let base = spawn_http_server(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789",
        ]);
        let err = fetch_bytes(&format!("{base}/big"), 5).unwrap_err();
        assert!(err.contains("exceeded limit"));
    }

    #[test]
    fn downloads_stream_to_file() {
        let base = spawn_http_server(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\nupdate-bytes",
        ]);
        let dest = std::env::temp_dir().join(format!(
            "audio-lock-http-test-{}.bin",
            std::process::id()
        ));
        download_to_file(&format!("{base}/asset.zip"), &dest, 1024).expect("download");
        let content = fs::read(&dest).expect("read downloaded file");
        let _ = fs::remove_file(&dest);
        assert_eq!(content, b"update-bytes");
    }
}
