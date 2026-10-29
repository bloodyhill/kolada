//! A WebSocket on Windows' own HTTP client (winhttp), for Quick Send's live
//! connection (see `quick`): proxies as the system has them, TLS by
//! Windows, keep-alive pings every 30 s sent by winhttp itself (a hibernating
//! server answers them without waking). One thread receives while another
//! sends, as winhttp allows.

use windows::Win32::Networking::WinHttp::{
    URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_INTERNET_SCHEME_HTTPS, WINHTTP_OPEN_REQUEST_FLAGS,
    WINHTTP_OPTION_RECEIVE_TIMEOUT, WINHTTP_OPTION_UPGRADE_TO_WEB_SOCKET, WINHTTP_OPTION_WEB_SOCKET_KEEPALIVE_INTERVAL,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WINHTTP_WEB_SOCKET_BINARY_FRAGMENT_BUFFER_TYPE,
    WINHTTP_WEB_SOCKET_BINARY_MESSAGE_BUFFER_TYPE, WINHTTP_WEB_SOCKET_BUFFER_TYPE, WINHTTP_WEB_SOCKET_CLOSE_BUFFER_TYPE,
    WINHTTP_WEB_SOCKET_UTF8_FRAGMENT_BUFFER_TYPE, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
    WinHttpSetTimeouts, WINHTTP_WEB_SOCKET_UTF8_MESSAGE_BUFFER_TYPE, WinHttpWebSocketClose, WinHttpWebSocketCompleteUpgrade, WinHttpWebSocketReceive, WinHttpWebSocketSend,
};
use windows::core::{PCWSTR, w};

/// The most one message may be: the largest sealed item and its frame.
const MOST: usize = 300 * 1024;

struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// An open WebSocket. Shared between the thread that receives and the one
/// that sends; its handles close when the last holder lets go.
pub struct Ws {
    socket: Handle,
    _request: Handle,
    _connect: Handle,
    _session: Handle,
}

// winhttp's WebSocket handle may be used from any thread, one receive and
// one send at a time.
unsafe impl Send for Ws {}
unsafe impl Sync for Ws {}

/// Why a socket did not open: the server's status and body (a refusal, in
/// `key=value` lines), or 0 and nothing when no answer came.
#[derive(Debug)]
pub struct Refused {
    pub status: u16,
    pub body: String,
}

impl Ws {
    /// Opens `url` (`wss://` or `ws://`) with `headers`.
    pub fn connect(url: &str, headers: &[(String, String)]) -> Result<Ws, Refused> {
        let none = || Refused { status: 0, body: String::new() };
        let http = url.replacen("wss://", "https://", 1).replacen("ws://", "http://", 1);
        let wide: Vec<u16> = http.encode_utf16().collect();
        let mut parts = URL_COMPONENTS {
            dwStructSize: size_of::<URL_COMPONENTS>() as u32,
            dwSchemeLength: u32::MAX,
            dwHostNameLength: u32::MAX,
            dwUrlPathLength: u32::MAX,
            dwExtraInfoLength: u32::MAX,
            ..Default::default()
        };
        unsafe { WinHttpCrackUrl(&wide, 0, &mut parts).map_err(|_| none())? };
        if parts.lpszHostName.is_null() {
            return Err(none());
        }
        let piece = |p: *mut u16, n: u32| unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec();
        let mut host = piece(parts.lpszHostName.0, parts.dwHostNameLength);
        host.push(0);
        let mut path = if parts.lpszUrlPath.is_null() { vec![b'/' as u16] } else { piece(parts.lpszUrlPath.0, parts.dwUrlPathLength + parts.dwExtraInfoLength) };
        path.push(0);
        let extra: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
        let extra: Vec<u16> = extra.encode_utf16().collect();
        let secure = parts.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS;
        unsafe {
            let session = Handle(WinHttpOpen(w!("Kolada"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
            if session.0.is_null() {
                return Err(none());
            }
            let _ = WinHttpSetTimeouts(session.0, 5_000, 5_000, 15_000, 15_000);
            let keepalive = 30_000u32.to_ne_bytes();
            let _ = WinHttpSetOption(Some(session.0), WINHTTP_OPTION_WEB_SOCKET_KEEPALIVE_INTERVAL, Some(&keepalive));
            let connect = Handle(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), parts.nPort, 0));
            if connect.0.is_null() {
                return Err(none());
            }
            let flags = if secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
            let request = Handle(WinHttpOpenRequest(connect.0, w!("GET"), PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags));
            if request.0.is_null() {
                return Err(none());
            }
            WinHttpSetOption(Some(request.0), WINHTTP_OPTION_UPGRADE_TO_WEB_SOCKET, None).map_err(|_| none())?;
            WinHttpSendRequest(request.0, (!extra.is_empty()).then_some(&extra[..]), None, 0, 0, 0).map_err(|_| none())?;
            WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|_| none())?;
            let (mut status, mut size) = (0u32, size_of::<u32>() as u32);
            WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut status as *mut u32 as *mut _), &mut size, std::ptr::null_mut())
                .map_err(|_| none())?;
            if status != 101 {
                return Err(Refused { status: status as u16, body: read_body(&request) });
            }
            let socket = Handle(WinHttpWebSocketCompleteUpgrade(request.0, None));
            if socket.0.is_null() {
                return Err(none());
            }
            // Waiting for the next message has no end: the pings keep it.
            let forever = 0u32.to_ne_bytes();
            let _ = WinHttpSetOption(Some(socket.0), WINHTTP_OPTION_RECEIVE_TIMEOUT, Some(&forever));
            Ok(Ws { socket, _request: request, _connect: connect, _session: session })
        }
    }

    /// Sends one binary message.
    pub fn send(&self, bytes: &[u8]) -> bool {
        unsafe { WinHttpWebSocketSend(self.socket.0, WINHTTP_WEB_SOCKET_BINARY_MESSAGE_BUFFER_TYPE, Some(bytes)) == 0 }
    }

    /// Sends the heartbeat: the server answers "pong" (a text message).
    pub fn ping(&self) -> bool {
        unsafe { WinHttpWebSocketSend(self.socket.0, WINHTTP_WEB_SOCKET_UTF8_MESSAGE_BUFFER_TYPE, Some(b"ping")) == 0 }
    }

    /// The next whole message (its fragments joined); None once closed.
    pub fn receive(&self) -> Option<Vec<u8>> {
        let mut message = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let (mut read, mut kind) = (0u32, WINHTTP_WEB_SOCKET_BUFFER_TYPE::default());
            let r = unsafe { WinHttpWebSocketReceive(self.socket.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut read, &mut kind) };
            if r != 0 || kind == WINHTTP_WEB_SOCKET_CLOSE_BUFFER_TYPE {
                return None;
            }
            message.extend_from_slice(&buf[..read as usize]);
            if message.len() > MOST {
                return None;
            }
            if kind != WINHTTP_WEB_SOCKET_BINARY_FRAGMENT_BUFFER_TYPE && kind != WINHTTP_WEB_SOCKET_UTF8_FRAGMENT_BUFFER_TYPE {
                return Some(message);
            }
        }
    }

    /// Closes it: a `receive` waiting on another thread returns None.
    pub fn close(&self) {
        unsafe {
            let _ = WinHttpWebSocketClose(self.socket.0, 1000, None, 0);
        }
    }
}

fn read_body(request: &Handle) -> String {
    let mut body = Vec::new();
    loop {
        let mut ready = 0u32;
        if unsafe { WinHttpQueryDataAvailable(request.0, &mut ready) }.is_err() || ready == 0 || body.len() > 4096 {
            break;
        }
        let at = body.len();
        body.resize(at + ready as usize, 0);
        let mut read = 0u32;
        if unsafe { WinHttpReadData(request.0, body[at..].as_mut_ptr() as *mut _, ready, &mut read) }.is_err() {
            break;
        }
        body.truncate(at + read as usize);
    }
    String::from_utf8_lossy(&body).into_owned()
}
