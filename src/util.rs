//! 共通の定数・ヘルパ(プラットフォーム非依存)。

/// WaitForSingleObject 互換の戻り値（WaitTsStream で使用）。
pub const WAIT_OBJECT_0: u32 = 0x0000_0000;
pub const WAIT_ABANDONED: u32 = 0x0000_0080;
pub const WAIT_TIMEOUT: u32 = 0x0000_0102;

/// Win32 BOOL 互換。IBonDriver の BOOL はどのプラットフォームでも int(4byte)
/// として扱う(EDCB/Windows 双方のホスト定義に一致)。
pub const TRUE: i32 = 1;
pub const FALSE: i32 = 0;

/// チューナ名。
pub const TUNER_NAME: &str = "BonDriver_Mirakurun";

/// HTTP の User-Agent (BonDriver_mirakc と同様に送る)。
pub const USER_AGENT: &str = "BonDriver_Mirakurun/2.1";

/// TSデータのサイズ (188 * 256)。
pub const TSDATASIZE: usize = 48128;

/// チューナ空間の最大数。
pub const SPACE_NUM: usize = 8;

/// 受信バッファに保持するTSチャンク数の上限。
/// 元実装の ASYNCBUFFSIZE = 0x200000 / TSDATASIZE * 2 と同等。
pub const ASYNCBUFFSIZE: usize = (0x0020_0000 / TSDATASIZE) * 2;

/// ビットレート計算間隔(ms)。BonDriver_mirakc は 1000ms。
pub const BITRATE_CALC_TIME_MS: u128 = 1000;

/// GetSignalLevel が返すビットレートの上限(Mbps)。BonDriver_mirakc と同じ。
pub const BITRATE_MAX_MBPS: f32 = 100.0;

/// UTF-8文字列を NUL 終端の UTF-16(ワイド文字列) に変換する。
///
/// Windows の `LPCWSTR`(UTF-16) と Linux ホストの `BON16CHAR`(UTF-16) は
/// どちらも 16bit なので、同じ表現を使い回せる。
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
