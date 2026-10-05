//! ストリーム通信用のトランスポート抽象。
//!
//! TCP(http) と Unix ドメインソケット(unix)の両方を同じインタフェースで
//! 扱う。Windows では Unix ソケットを使わないため `#[cfg(unix)]` で除外する。

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::config::Config;

/// TCP もしくは Unix ドメインソケットを包む。
pub enum Stream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.read(buf),
            #[cfg(unix)]
            Stream::Unix(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.write(buf),
            #[cfg(unix)]
            Stream::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Tcp(s) => s.flush(),
            #[cfg(unix)]
            Stream::Unix(s) => s.flush(),
        }
    }
}

impl Stream {
    pub fn set_read_timeout(&self, dur: Option<Duration>) {
        match self {
            Stream::Tcp(s) => {
                let _ = s.set_read_timeout(dur);
            }
            #[cfg(unix)]
            Stream::Unix(s) => {
                let _ = s.set_read_timeout(dur);
            }
        }
    }

    /// 接続を閉じる(受信スレッドのブロック解除に使う)。
    pub fn shutdown_both(&self) {
        match self {
            Stream::Tcp(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            #[cfg(unix)]
            Stream::Unix(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        }
    }

    /// 複製。受信スレッドへ渡し、別スレッドから shutdown するために使う。
    pub fn try_clone(&self) -> std::io::Result<Stream> {
        match self {
            Stream::Tcp(s) => s.try_clone().map(Stream::Tcp),
            #[cfg(unix)]
            Stream::Unix(s) => s.try_clone().map(Stream::Unix),
        }
    }

    /// Nagle を無効化する(TCP のみ。失敗は無視)。
    pub fn set_nodelay(&self, on: bool) {
        match self {
            Stream::Tcp(s) => {
                let _ = s.set_nodelay(on);
            }
            #[cfg(unix)]
            Stream::Unix(_) => {}
        }
    }
}

/// 設定に従って接続を確立する。失敗した場合は None。
///
/// - `SERVER_TYPE="unix"` かつ `SERVER_SOCKPATH` 指定時: Unix ドメインソケット。
/// - それ以外: `SERVER_HOST:SERVER_PORT` への TCP 接続。
///   ホスト名・IPv4/IPv6 いずれも `ToSocketAddrs` で解決する。
pub fn connect(cfg: &Config) -> Option<Stream> {
    if cfg.use_unix_socket() {
        #[cfg(unix)]
        {
            return std::os::unix::net::UnixStream::connect(&cfg.server_sockpath)
                .ok()
                .map(Stream::Unix);
        }
        #[cfg(not(unix))]
        {
            // Windows では Unix ソケット非対応。TCP へフォールバックする。
            eprintln!(
                "BonDriver_Mirakurun: SERVER_TYPE=unix is not supported on this platform; falling back to TCP"
            );
        }
    }

    let addrs = (cfg.server_host.as_str(), cfg.port_num())
        .to_socket_addrs()
        .ok()?;
    for addr in addrs {
        if let Ok(s) = TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
            return Some(Stream::Tcp(s));
        }
    }
    None
}
