//! チューナ本体(元実装の CBonTuner 相当)。
//!
//! TVTest/EDCB から呼ばれる各APIの実体。チャンネル選局時に Mirakurun/mirakc の
//! stream API へ HTTP GET を発行し、バックグラウンドスレッドでTS(TLV)データを
//! 受信してキューに積む。
//!
//! 元実装は Push/Pop の2スレッド + オーバーラップドI/O だったが、ここでは
//! 1本の受信スレッド + Mutex/Condvar のキューという等価でより素直な構成にしている。
//!
//! 通信路は TCP と Unix ドメインソケット(`SERVER_TYPE=unix`)の両方に対応する。

use std::collections::VecDeque;
use std::ffi::c_void;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::transport::{self, Stream};
use crate::util::{
    ASYNCBUFFSIZE, BITRATE_CALC_TIME_MS, BITRATE_MAX_MBPS, FALSE, TRUE, TSDATASIZE, TUNER_NAME,
    USER_AGENT, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT, to_wide,
};
use crate::{CONFIG, with_channels};

/// 受信スレッドとホスト側スレッドで共有する状態。
struct Shared {
    queue: Mutex<VecDeque<Vec<u8>>>,
    cond: Condvar,
    recv_bytes: AtomicU64,
    running: AtomicBool,
}

/// 1つのストリーム受信セッション。
struct StreamCtx {
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
    /// 受信スレッドの read をブロック解除するための複製ストリーム。
    shutdown: Stream,
}

/// チューナの状態。
pub struct Tuner {
    open: bool,
    cur_space: u32,
    cur_channel: u32,

    stream: Option<StreamCtx>,
    /// GetTsStream(ポインタ版)で返すバッファの保持先。
    last_buf: Vec<u8>,
    /// EnumTuningSpace / EnumChannelName が返すワイド文字列の保持先。
    wbuf: Vec<u16>,
    /// チューナ名のワイド文字列。
    name_wide: Vec<u16>,

    /// 多重オープン検出用の名前付きミューテックス(Windows のみ)。
    mutex_handle: *mut c_void,

    bitrate: f32,
    last_calc: Instant,
}

// 生ポインタ(HANDLE)を含むが、Tuner はホスト側スレッドからのみ操作され、
// 受信スレッドへ渡すのは Arc<Shared> のみであるため安全に扱える。
unsafe impl Send for Tuner {}

impl Tuner {
    pub fn new() -> Tuner {
        Tuner {
            open: false,
            cur_space: 0,
            cur_channel: 0xFFFF_FFFF,
            stream: None,
            last_buf: Vec::new(),
            wbuf: Vec::new(),
            name_wide: to_wide(TUNER_NAME),
            mutex_handle: std::ptr::null_mut(),
            bitrate: 0.0,
            last_calc: Instant::now(),
        }
    }

    // ---- IBonDriver ----------------------------------------------------

    pub fn open_tuner(&mut self) -> i32 {
        // チャンネル情報が取得できていなければ失敗。
        if with_channels(|c| c.is_empty()) {
            return FALSE;
        }

        self.open = true;
        TRUE
    }

    pub fn close_tuner(&mut self) {
        // ストリーム停止。
        if let Some(mut ctx) = self.stream.take() {
            ctx.shared.running.store(false, Ordering::SeqCst);
            ctx.shutdown.shutdown_both();
            ctx.shared.cond.notify_all();
            if let Some(h) = ctx.handle.take() {
                let _ = h.join();
            }
        }

        // チャンネル初期化。
        self.cur_space = 0;
        self.cur_channel = 0xFFFF_FFFF;

        // ミューテックス開放。
        destroy_sole_instance(self.mutex_handle);
        self.mutex_handle = std::ptr::null_mut();

        self.bitrate = 0.0;
    }

    pub fn set_channel_byte(&mut self, ch: u8) -> i32 {
        // 元実装: SetChannel((DWORD)0, (DWORD)bCh - 13)
        self.set_channel(0, (ch as u32).wrapping_sub(13))
    }

    pub fn get_signal_level(&mut self) -> f32 {
        self.calc_bitrate();
        self.bitrate
    }

    pub fn wait_ts_stream(&mut self, timeout: u32) -> u32 {
        let ctx = match &self.stream {
            Some(c) => c,
            None => return WAIT_ABANDONED,
        };
        if !ctx.shared.running.load(Ordering::SeqCst) {
            return WAIT_ABANDONED;
        }

        let mut guard = ctx.shared.queue.lock().unwrap();
        if guard.is_empty() {
            if timeout == 0 {
                // INFINITE 相当: データが来るかクローズされるまで待つ。
                while guard.is_empty() && ctx.shared.running.load(Ordering::SeqCst) {
                    let (g, _) = ctx
                        .shared
                        .cond
                        .wait_timeout(guard, Duration::from_millis(1000))
                        .unwrap();
                    guard = g;
                }
            } else {
                let (g, _) = ctx
                    .shared
                    .cond
                    .wait_timeout(guard, Duration::from_millis(timeout as u64))
                    .unwrap();
                guard = g;
            }
        }

        if !ctx.shared.running.load(Ordering::SeqCst) {
            return WAIT_ABANDONED;
        }
        if guard.is_empty() {
            WAIT_TIMEOUT
        } else {
            WAIT_OBJECT_0
        }
    }

    pub fn get_ready_count(&self) -> u32 {
        match &self.stream {
            Some(ctx) => ctx.shared.queue.lock().unwrap().len() as u32,
            None => 0,
        }
    }

    /// GetTsStream(ポインタ版)。返したポインタは次回呼び出しまで有効。
    pub fn get_ts_stream_ptr(
        &mut self,
        ppdst: *mut *mut u8,
        psize: *mut u32,
        premain: *mut u32,
    ) -> i32 {
        let ctx = match &self.stream {
            Some(c) => c,
            None => return FALSE,
        };

        let popped = {
            let mut guard = ctx.shared.queue.lock().unwrap();
            let buf = guard.pop_front();
            let remain = guard.len() as u32;
            buf.map(|b| (b, remain))
        };

        unsafe {
            match popped {
                Some((buf, remain)) => {
                    self.last_buf = buf;
                    *ppdst = self.last_buf.as_mut_ptr();
                    *psize = self.last_buf.len() as u32;
                    *premain = remain;
                }
                None => {
                    // データ無し。ホストが *ppdst を参照しても安全なよう null を返す。
                    *ppdst = std::ptr::null_mut();
                    *psize = 0;
                    *premain = 0;
                }
            }
        }
        TRUE
    }

    /// GetTsStream(コピー版)。
    pub fn get_ts_stream_copy(&mut self, pdst: *mut u8, psize: *mut u32, premain: *mut u32) -> i32 {
        let mut src: *mut u8 = std::ptr::null_mut();
        let ret = self.get_ts_stream_ptr(&mut src, psize, premain);
        if ret == TRUE {
            let size = unsafe { *psize } as usize;
            if size > 0 && !src.is_null() && !pdst.is_null() {
                unsafe {
                    std::ptr::copy_nonoverlapping(src, pdst, size);
                }
            }
        }
        ret
    }

    pub fn purge_ts_stream(&mut self) {
        if let Some(ctx) = &self.stream {
            ctx.shared.queue.lock().unwrap().clear();
        }
    }

    // ---- IBonDriver2 ---------------------------------------------------

    pub fn tuner_name_ptr(&self) -> *const u16 {
        self.name_wide.as_ptr()
    }

    pub fn is_tuner_opening(&self) -> i32 {
        is_sole_instance(self.open)
    }

    pub fn enum_tuning_space(&mut self, space: u32) -> *const u16 {
        match with_channels(|c| c.enum_tuning_space(space).map(to_wide)) {
            Some(w) => {
                self.wbuf = w;
                self.wbuf.as_ptr()
            }
            None => std::ptr::null(),
        }
    }

    pub fn enum_channel_name(&mut self, space: u32, channel: u32) -> *const u16 {
        match with_channels(|c| c.enum_channel_name(space, channel).map(to_wide)) {
            Some(w) => {
                self.wbuf = w;
                self.wbuf.as_ptr()
            }
            None => std::ptr::null(),
        }
    }

    pub fn set_channel(&mut self, space: u32, channel: u32) -> i32 {
        // 対象チャンネルの情報を解決する。
        let entry = match with_channels(|c| c.entry(space, channel).cloned()) {
            Some(e) => e,
            None => return FALSE,
        };

        let cfg = match CONFIG.get() {
            Some(c) => c,
            None => return FALSE,
        };

        // 一旦クローズ。
        self.close_tuner();

        // 接続先URLを構築する(BonDriver_mirakc と同じエンドポイント)。
        //   SERVICE_SPLIT=0: /api/channels/{type}/{channel}/stream
        //   SERVICE_SPLIT=1: /api/services/{id}/stream
        let path = if cfg.service_split == 1 {
            format!(
                "/api/services/{}/stream?decode={}",
                entry.service_id.unwrap_or(0),
                cfg.decode_b25
            )
        } else {
            format!(
                "/api/channels/{}/{}/stream?decode={}",
                entry.type_, entry.channel, cfg.decode_b25
            )
        };

        // X-Mirakc-Tuner は channel stream のみで有効。エントリにチューナー名が
        // 無ければ設定値(GLOBAL.TUNER)を使う。BS4K 等でチューナーを厳密に
        // 固定したい場合に利用する(mirakc-BS4K 独自機能)。
        let tuner = if entry.tuner.is_empty() {
            cfg.tuner.as_str()
        } else {
            entry.tuner.as_str()
        };
        let tuner_header = sanitize_header_value(tuner)
            .map(|t| format!("X-Mirakc-Tuner: {t}\r\n"))
            .unwrap_or_default();

        // 接続〜ストリーム確立。BonDriver_mirakc 同様、非 200 の場合は 500ms
        // 空けて 1 回だけ再試行する(多重オープン/チューナー競合対策)。
        let mut established = None;
        for attempt in 0..2 {
            match open_stream(cfg, &path, &tuner_header) {
                Ok(ctx) => {
                    established = Some(ctx);
                    break;
                }
                Err(code) => {
                    if attempt == 0 {
                        std::thread::sleep(Duration::from_millis(500));
                    } else {
                        // ストリームは得られなかったが、BonDriver_mirakc と同様に
                        // 選局は完了扱いにする(GetReadyCount=0 のまま)。
                        eprintln!(
                            "BonDriver_Mirakurun: stream unavailable (status {:?}) for {}",
                            code, path
                        );
                        self.cur_space = space;
                        self.cur_channel = channel;
                        return TRUE;
                    }
                }
            }
        }

        // 多重オープン検出用ミューテックス作成(Windows のみ)。
        self.mutex_handle = create_sole_instance();

        // チャンネル情報更新。
        self.cur_space = space;
        self.cur_channel = channel;

        // 受信セッションを設定してからキューを空にする。
        self.stream = established;
        self.purge_ts_stream();
        TRUE
    }

    pub fn get_cur_space(&self) -> u32 {
        self.cur_space
    }

    pub fn get_cur_channel(&self) -> u32 {
        self.cur_channel
    }

    // ---- 内部処理 ------------------------------------------------------

    fn calc_bitrate(&mut self) {
        let span = self.last_calc.elapsed().as_millis();
        if span >= BITRATE_CALC_TIME_MS {
            let bytes = match &self.stream {
                Some(ctx) => ctx.shared.recv_bytes.swap(0, Ordering::SeqCst),
                None => 0,
            };
            // Mbps 換算。TLV/TS いずれもバイト量ベースで算出する。
            let mbps = ((bytes as f64 * 8.0 * 1000.0) / (span as f64 * 1024.0 * 1024.0)) as f32;
            // BonDriver_mirakc と同様に 100Mbps で頭打ちにする。
            self.bitrate = mbps.min(BITRATE_MAX_MBPS);
            self.last_calc = Instant::now();
        }
    }
}

impl Drop for Tuner {
    fn drop(&mut self) {
        self.close_tuner();
    }
}

/// キューへ1チャンク積む(上限超過時は古いものから捨てる)。
fn enqueue(shared: &Shared, chunk: Vec<u8>) {
    if chunk.is_empty() {
        return;
    }
    shared
        .recv_bytes
        .fetch_add(chunk.len() as u64, Ordering::SeqCst);
    let mut guard = shared.queue.lock().unwrap();
    while guard.len() >= ASYNCBUFFSIZE {
        guard.pop_front();
    }
    guard.push_back(chunk);
    drop(guard);
    shared.cond.notify_one();
}

/// ストリーム接続を確立して受信スレッドを起動する。
///
/// レスポンスのステータス行・ヘッダをここで読み切ってボディ先頭の残りを
/// 取り出し、受信スレッドへ渡す(BonDriver_mirakc の WinHTTP 相当)。
/// 非 200 の場合や接続失敗の場合は `Err(Some(status))` / `Err(None)` を返す。
fn open_stream(cfg: &Config, path: &str, tuner_header: &str) -> Result<StreamCtx, Option<u16>> {
    let mut stream = transport::connect(cfg).ok_or(None)?;
    stream.set_nodelay(true);

    let request = format!(
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: {ua}\r\nX-Mirakurun-Priority: {pri}\r\n{tuner}Connection: close\r\n\r\n",
        host = cfg.server_host,
        ua = USER_AGENT,
        pri = cfg.priority,
        tuner = tuner_header,
    );
    stream.write_all(request.as_bytes()).map_err(|_| None)?;

    // ステータス確認 + ヘッダ読み飛ばし。
    let leftover = read_stream_header(&mut stream)?;

    // ブロック解除用の複製。
    let shutdown = stream.try_clone().map_err(|_| None)?;

    let shared = Arc::new(Shared {
        queue: Mutex::new(VecDeque::new()),
        cond: Condvar::new(),
        recv_bytes: AtomicU64::new(0),
        running: AtomicBool::new(true),
    });
    let shared_thread = Arc::clone(&shared);
    let handle = std::thread::spawn(move || reader_loop(stream, shared_thread, leftover));

    Ok(StreamCtx {
        shared,
        handle: Some(handle),
        shutdown,
    })
}

/// レスポンスのステータス行+ヘッダを読み、200 以外なら Err(Some(status))。
/// 200 ならヘッダ以降の残りボディを返す。
fn read_stream_header(stream: &mut Stream) -> Result<Vec<u8>, Option<u16>> {
    /// ヘッダとして受け付ける最大サイズ。超えたら異常応答とみなす。
    const MAX_HEADER_SIZE: usize = 64 * 1024;

    let mut header = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = crate::http::find_subslice(&header, b"\r\n\r\n") {
            break pos;
        }
        if header.len() > MAX_HEADER_SIZE {
            return Err(None);
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err(None), // ヘッダ完結前に切断
            Ok(n) => header.extend_from_slice(&chunk[..n]),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => return Err(None),
        }
    };

    let status = parse_status_code(&header[..header_end]).ok_or(None)?;
    if status != 200 {
        return Err(Some(status));
    }
    Ok(header[header_end + 4..].to_vec())
}

/// "HTTP/1.1 200 OK" のステータスコードを取り出す。
pub(crate) fn parse_status_code(head: &[u8]) -> Option<u16> {
    let line_end = head
        .iter()
        .position(|&b| b == b'\r' || b == b'\n')
        .unwrap_or(head.len());
    let line = std::str::from_utf8(&head[..line_end]).ok()?;
    line.split_whitespace().nth(1)?.parse::<u16>().ok()
}

/// ヘッダ値として安全な文字列のみを返す(CR/LF 等の注入を除去)。
pub(crate) fn sanitize_header_value(v: &str) -> Option<String> {
    let t = v.trim();
    if t.is_empty() || t.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
        return None;
    }
    Some(t.to_string())
}

/// 受信スレッド本体。ストリームからデータを読み続けてキューへ積む。
///
/// ステータス行・ヘッダは `read_stream_header` で読み飛ばし済みで、`leftover`
/// にボディ先頭の残りが入っている。BS4K(MMT/TLV)の場合は 188byte TS ではなく
/// TLV パケットが流れてくるが、内容に依存せずバイト列のまま中継する。
fn reader_loop(mut stream: Stream, shared: Arc<Shared>, leftover: Vec<u8>) {
    if !leftover.is_empty() {
        enqueue(&shared, leftover);
    }

    let mut buf = vec![0u8; TSDATASIZE];
    while shared.running.load(Ordering::SeqCst) {
        match stream.read(&mut buf) {
            Ok(0) => break, // 切断
            Ok(n) => enqueue(&shared, buf[..n].to_vec()),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => break,
        }
    }
    shared.running.store(false, Ordering::SeqCst);
    shared.cond.notify_all();
}

// ---- 多重オープン検出 --------------------------------------------------

/// 名前付きミューテックスを作成して保持する(Windows)。
#[cfg(windows)]
fn create_sole_instance() -> *mut c_void {
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name = to_wide(TUNER_NAME);
    unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) as *mut c_void }
}

#[cfg(not(windows))]
fn create_sole_instance() -> *mut c_void {
    std::ptr::null_mut()
}

/// 名前付きミューテックスを開放する(Windows)。
#[cfg(windows)]
fn destroy_sole_instance(handle: *mut c_void) {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Threading::ReleaseMutex;
    if handle.is_null() {
        return;
    }
    unsafe {
        let h = handle as HANDLE;
        ReleaseMutex(h);
        CloseHandle(h);
    }
}

#[cfg(not(windows))]
fn destroy_sole_instance(_handle: *mut c_void) {}

/// 既にチューナーが開かれているか。
///
/// Windows では元実装同様、名前付きミューテックスの存在で判定する。
/// Linux 等では名前付きミューテックスが無いため、自身の open 状態を返す。
#[cfg(windows)]
fn is_sole_instance(_open: bool) -> i32 {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::OpenMutexW;
    const MUTEX_ALL_ACCESS: u32 = 0x001F_0001;

    let name = to_wide(TUNER_NAME);
    let handle = unsafe { OpenMutexW(MUTEX_ALL_ACCESS, 0, name.as_ptr()) };
    if !handle.is_null() {
        unsafe {
            CloseHandle(handle);
        }
        TRUE
    } else {
        FALSE
    }
}

#[cfg(not(windows))]
fn is_sole_instance(open: bool) -> i32 {
    if open { TRUE } else { FALSE }
}
