//! BonDriver_Mirakurun (Rust 実装)
//!
//! TVTest / EDCB 等から利用される BonDriver (共有ライブラリ)。`CreateBonDriver`
//! (C++互換 vtable) と `CreateBonStruct` (C互換構造体、コンパイラ非依存) の
//! 両方をエクスポートし、Windows / Linux いずれのホストからも利用できる。
//!
//! - Windows x64: MSVC x64 の vtable レイアウト + RTTI をエミュレートし、
//!   TVTest の `dynamic_cast<IBonDriver2*>` に対応する。
//! - Windows x86(32bit): メンバ関数が `__thiscall`(this を ECX で渡す)に
//!   なるため、vtable スロットを `extern "thiscall"` で構築する。RTTI は
//!   x86 形式(絶対アドレス・COL 20バイト)でエミュレートする。
//! - Linux: EDCB は `CreateBonStruct` を優先するため、コンパイラ非依存の
//!   C互換構造体で受け渡す(`dynamic_cast` を必要としない)。

// クレート名(BonDriver_Mirakurun)は意図的に CamelCase のため警告を抑制する。
#![allow(non_snake_case)]

/// Windows 32bit(x86)ではメンバ関数の呼び出し規約が `__thiscall`
/// (this を ECX で渡す)になる。それ以外(x64 / Linux)は `extern "C"`
/// (x64 では this が第1引数と一致)。
///
/// `$abi:literal` にはマクロ展開結果を直接渡せないため、呼び出し規約を
/// 埋め込んだ形で `define_vtable!` を展開する。
#[cfg(all(windows, target_arch = "x86"))]
macro_rules! bon_vtable {
    () => {
        define_vtable!("thiscall");
    };
}
#[cfg(not(all(windows, target_arch = "x86")))]
macro_rules! bon_vtable {
    () => {
        define_vtable!("C");
    };
}

mod channels;
mod config;
mod http;
#[cfg(windows)]
mod rtti;
mod transport;
mod tuner;
mod util;

#[cfg(test)]
mod tests;

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{OnceLock, RwLock};

#[cfg(windows)]
use std::sync::atomic::AtomicIsize;

use channels::Channels;
use config::Config;
use tuner::Tuner;

// ---- グローバル状態 ----------------------------------------------------

/// DllMain で読み込んだ設定。
static CONFIG: OnceLock<Config> = OnceLock::new();
/// CreateBonDriver 時に取得するチャンネル情報。
static CHANNELS: RwLock<Option<Channels>> = RwLock::new(None);
/// 多重生成を防ぐためのシングルトン(元実装の m_pThis 相当)。
static INSTANCE: AtomicPtr<BonObject> = AtomicPtr::new(std::ptr::null_mut());
/// インスタンス生成/破棄を直列化するロック。ホストが同時に
/// CreateBonDriver/CreateBonStruct/Release を呼んでも安全にする。
static INSTANCE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// DLLのモジュールハンドル(INIパス解決用、Windows のみ)。
#[cfg(windows)]
static HMODULE_VALUE: AtomicIsize = AtomicIsize::new(0);

/// グローバルなチャンネル情報へ読み取りアクセスする。
pub(crate) fn with_channels<R>(f: impl FnOnce(&Channels) -> R) -> R {
    let guard = CHANNELS.read().unwrap();
    match guard.as_ref() {
        Some(c) => f(c),
        None => {
            // 未取得時は空とみなす。
            let empty = Channels::default();
            f(&empty)
        }
    }
}

// ---- IBonDriver2 vtable レイアウト -------------------------------------

/// IBonDriver2 の vtable と、そこへ載せる thunk 群を生成する。
///
/// スロット順はプラットフォームで異なる:
/// - MSVC(x86/x64)は同名オーバーロードを「宣言と逆順」で配置するため、
///   GetTsStream は slot6=ポインタ版, slot7=コピー版。
/// - Itanium ABI(Linux)は宣言順のため slot6=コピー版, slot7=ポインタ版。
///
/// 呼び出し規約は `$abi` で切り替える(Windows x86 は "thiscall")。
macro_rules! define_vtable {
    ($abi:literal) => {
        #[repr(C)]
        pub struct IBonDriver2Vtbl {
            // IBonDriver
            open_tuner: extern $abi fn(*mut BonObject) -> i32,
            close_tuner: extern $abi fn(*mut BonObject),
            set_channel_byte: extern $abi fn(*mut BonObject, u8) -> i32,
            get_signal_level: extern $abi fn(*mut BonObject) -> f32,
            wait_ts_stream: extern $abi fn(*mut BonObject, u32) -> u32,
            get_ready_count: extern $abi fn(*mut BonObject) -> u32,
            // slot6/7: GetTsStream のオーバーロード(順序は ABI 依存)。
            #[cfg(windows)]
            get_ts_stream_ptr: extern $abi fn(*mut BonObject, *mut *mut u8, *mut u32, *mut u32) -> i32,
            #[cfg(windows)]
            get_ts_stream: extern $abi fn(*mut BonObject, *mut u8, *mut u32, *mut u32) -> i32,
            #[cfg(not(windows))]
            get_ts_stream: extern $abi fn(*mut BonObject, *mut u8, *mut u32, *mut u32) -> i32,
            #[cfg(not(windows))]
            get_ts_stream_ptr: extern $abi fn(*mut BonObject, *mut *mut u8, *mut u32, *mut u32) -> i32,
            purge_ts_stream: extern $abi fn(*mut BonObject),
            release: extern $abi fn(*mut BonObject),
            // IBonDriver2
            get_tuner_name: extern $abi fn(*mut BonObject) -> *const u16,
            is_tuner_opening: extern $abi fn(*mut BonObject) -> i32,
            enum_tuning_space: extern $abi fn(*mut BonObject, u32) -> *const u16,
            enum_channel_name: extern $abi fn(*mut BonObject, u32, u32) -> *const u16,
            set_channel_2: extern $abi fn(*mut BonObject, u32, u32) -> i32,
            get_cur_space: extern $abi fn(*mut BonObject) -> u32,
            get_cur_channel: extern $abi fn(*mut BonObject) -> u32,
        }

        extern $abi fn thunk_open_tuner(this: *mut BonObject) -> i32 {
            guard(util::FALSE, || unsafe { tuner(this).open_tuner() })
        }
        extern $abi fn thunk_close_tuner(this: *mut BonObject) {
            guard((), || unsafe { tuner(this).close_tuner() })
        }
        extern $abi fn thunk_set_channel_byte(this: *mut BonObject, ch: u8) -> i32 {
            guard(util::FALSE, || unsafe { tuner(this).set_channel_byte(ch) })
        }
        extern $abi fn thunk_get_signal_level(this: *mut BonObject) -> f32 {
            guard(0.0, || unsafe { tuner(this).get_signal_level() })
        }
        extern $abi fn thunk_wait_ts_stream(this: *mut BonObject, timeout: u32) -> u32 {
            guard(util::WAIT_ABANDONED, || unsafe {
                tuner(this).wait_ts_stream(timeout)
            })
        }
        extern $abi fn thunk_get_ready_count(this: *mut BonObject) -> u32 {
            guard(0, || unsafe { tuner(this).get_ready_count() })
        }
        extern $abi fn thunk_get_ts_stream(
            this: *mut BonObject,
            pdst: *mut u8,
            psize: *mut u32,
            premain: *mut u32,
        ) -> i32 {
            guard(util::FALSE, || unsafe {
                tuner(this).get_ts_stream_copy(pdst, psize, premain)
            })
        }
        extern $abi fn thunk_get_ts_stream_ptr(
            this: *mut BonObject,
            ppdst: *mut *mut u8,
            psize: *mut u32,
            premain: *mut u32,
        ) -> i32 {
            guard(util::FALSE, || unsafe {
                tuner(this).get_ts_stream_ptr(ppdst, psize, premain)
            })
        }
        extern $abi fn thunk_purge_ts_stream(this: *mut BonObject) {
            guard((), || unsafe { tuner(this).purge_ts_stream() })
        }
        extern $abi fn thunk_release(this: *mut BonObject) {
            guard((), || release_instance(this))
        }
        extern $abi fn thunk_get_tuner_name(this: *mut BonObject) -> *const u16 {
            guard(std::ptr::null(), || unsafe { tuner(this).tuner_name_ptr() })
        }
        extern $abi fn thunk_is_tuner_opening(this: *mut BonObject) -> i32 {
            guard(util::FALSE, || unsafe { tuner(this).is_tuner_opening() })
        }
        extern $abi fn thunk_enum_tuning_space(this: *mut BonObject, space: u32) -> *const u16 {
            guard(std::ptr::null(), || unsafe {
                tuner(this).enum_tuning_space(space)
            })
        }
        extern $abi fn thunk_enum_channel_name(
            this: *mut BonObject,
            space: u32,
            channel: u32,
        ) -> *const u16 {
            guard(std::ptr::null(), || unsafe {
                tuner(this).enum_channel_name(space, channel)
            })
        }
        extern $abi fn thunk_set_channel_2(this: *mut BonObject, space: u32, channel: u32) -> i32 {
            guard(util::FALSE, || unsafe {
                tuner(this).set_channel(space, channel)
            })
        }
        extern $abi fn thunk_get_cur_space(this: *mut BonObject) -> u32 {
            guard(0, || unsafe { tuner(this).get_cur_space() })
        }
        extern $abi fn thunk_get_cur_channel(this: *mut BonObject) -> u32 {
            guard(0, || unsafe { tuner(this).get_cur_channel() })
        }

        pub static VTABLE: IBonDriver2Vtbl = IBonDriver2Vtbl {
            open_tuner: thunk_open_tuner,
            close_tuner: thunk_close_tuner,
            set_channel_byte: thunk_set_channel_byte,
            get_signal_level: thunk_get_signal_level,
            wait_ts_stream: thunk_wait_ts_stream,
            get_ready_count: thunk_get_ready_count,
            get_ts_stream_ptr: thunk_get_ts_stream_ptr,
            get_ts_stream: thunk_get_ts_stream,
            purge_ts_stream: thunk_purge_ts_stream,
            release: thunk_release,
            get_tuner_name: thunk_get_tuner_name,
            is_tuner_opening: thunk_is_tuner_opening,
            enum_tuning_space: thunk_enum_tuning_space,
            enum_channel_name: thunk_enum_channel_name,
            set_channel_2: thunk_set_channel_2,
            get_cur_space: thunk_get_cur_space,
            get_cur_channel: thunk_get_cur_channel,
        };
    };
}

bon_vtable!();

/// CreateBonDriver が返すオブジェクト。
/// 先頭に vtable ポインタを置くことで C++ オブジェクトと同一レイアウトになる。
#[repr(C)]
struct BonObject {
    vtable: *const IBonDriver2Vtbl,
    tuner: *mut Tuner,
    /// CreateBonStruct 用に確保した C 互換構造体の先頭ポインタ(未使用時 null)。
    holder: *mut c_void,
    /// 解放済みフラグ。二重 Release での二重解放を防ぐ。
    released: bool,
}

/// RTTI 付きブロックへ複製した vtable(TVTest の dynamic_cast 用)。
/// 一度構築したら DLL の寿命中使い回す。
#[cfg(windows)]
static VTABLE_WITH_RTTI: OnceLock<VtblPtr> = OnceLock::new();

/// 生ポインタを OnceLock に入れるためのラッパ。
/// 参照先は不変データのためスレッド間で共有しても安全。
#[cfg(windows)]
struct VtblPtr(*const IBonDriver2Vtbl);
#[cfg(windows)]
unsafe impl Send for VtblPtr {}
#[cfg(windows)]
unsafe impl Sync for VtblPtr {}

/// CreateBonDriver が返す vtable ポインタを取得する。
///
/// Windows の TVTest は `dynamic_cast<IBonDriver2*>` を行うため、RTTI 付きの
/// 複製 vtable を返す必要がある。Linux の EDCB は `CreateBonStruct` を使うため
/// 静的 vtable のままでよい。
#[cfg(windows)]
fn vtable_ptr() -> *const IBonDriver2Vtbl {
    VTABLE_WITH_RTTI
        .get_or_init(|| {
            let p = unsafe {
                rtti::build_vtable_with_rtti(
                    &VTABLE as *const IBonDriver2Vtbl as *const u8,
                    std::mem::size_of::<IBonDriver2Vtbl>(),
                )
            };
            VtblPtr(p as *const IBonDriver2Vtbl)
        })
        .0
}

#[cfg(not(windows))]
fn vtable_ptr() -> *const IBonDriver2Vtbl {
    &VTABLE
}

// ---- thunk: vtable から Rust メソッドへの橋渡し ------------------------

/// `this` から Tuner の可変参照を得る。
#[inline]
unsafe fn tuner<'a>(this: *mut BonObject) -> &'a mut Tuner {
    unsafe { &mut *(*this).tuner }
}

/// FFI境界をまたぐ panic を遮断するヘルパ。
#[inline]
fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(default)
}

/// インスタンス開放。Box を破棄し、シングルトンをクリアする。
///
/// 二重 Release は `released` フラグで無害化する(ホストが vtable と
/// C 互換構造体の双方から Release を呼ぶケースがある)。
fn release_instance(this: *mut BonObject) {
    if this.is_null() {
        return;
    }
    let _lock = INSTANCE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        if (*this).released {
            return;
        }
        (*this).released = true;
        INSTANCE
            .compare_exchange(
                this,
                std::ptr::null_mut(),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .ok();
        let obj = Box::from_raw(this);
        // CreateBonStruct 用の C 互換構造体も開放する。
        if !obj.holder.is_null() {
            drop(Box::from_raw(obj.holder as *mut BonStruct2));
        }
        // Tuner の Drop で close_tuner が呼ばれる。
        drop(Box::from_raw(obj.tuner));
    }
}

/// シングルトンの BonObject を生成(既存があればそれを返す)。
/// 呼び出し側が `INSTANCE_LOCK` を保持していること。
fn create_instance_locked() -> *mut BonObject {
    let existing = INSTANCE.load(Ordering::SeqCst);
    if !existing.is_null() {
        return existing;
    }

    // チャンネル情報を取得する(元実装の InitChannel 相当)。
    if let Some(cfg) = CONFIG.get() {
        let ch = Channels::fetch(cfg);
        if let Ok(mut guard) = CHANNELS.write() {
            *guard = Some(ch);
        }
    }

    let tuner = Box::into_raw(Box::new(Tuner::new()));
    let obj = Box::into_raw(Box::new(BonObject {
        vtable: vtable_ptr(),
        tuner,
        holder: std::ptr::null_mut(),
        released: false,
    }));
    INSTANCE.store(obj, Ordering::SeqCst);
    obj
}

// ---- エクスポート関数 --------------------------------------------------

/// BonDriver インスタンスを生成する(C++ vtable 版)。既存があれば同じものを返す。
#[unsafe(no_mangle)]
pub extern "C" fn CreateBonDriver() -> *mut c_void {
    guard(std::ptr::null_mut(), || {
        let _lock = INSTANCE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        create_instance_locked() as *mut c_void
    })
}

// ---- CreateBonStruct: コンパイラ非依存の C 互換構造体 ------------------

/// EDCB の `STRUCT_IBONDRIVER` と同一レイアウト。
#[repr(C)]
struct BonStruct1 {
    ctx: *mut c_void,
    end: *const c_void,
    f00: extern "C" fn(*mut c_void) -> i32,
    f01: extern "C" fn(*mut c_void),
    f02: extern "C" fn(*mut c_void, u8) -> i32,
    f03: extern "C" fn(*mut c_void) -> f32,
    f04: extern "C" fn(*mut c_void, u32) -> u32,
    f05: extern "C" fn(*mut c_void) -> u32,
    f06: extern "C" fn(*mut c_void, *mut u8, *mut u32, *mut u32) -> i32,
    f07: extern "C" fn(*mut c_void, *mut *mut u8, *mut u32, *mut u32) -> i32,
    f08: extern "C" fn(*mut c_void),
    f09: extern "C" fn(*mut c_void),
}

/// EDCB の `STRUCT_IBONDRIVER2` と同一レイアウト(IBonDriver + IBonDriver2)。
#[repr(C)]
struct BonStruct2 {
    base: BonStruct1,
    f10: extern "C" fn(*mut c_void) -> *const u16,
    f11: extern "C" fn(*mut c_void) -> i32,
    f12: extern "C" fn(*mut c_void, u32) -> *const u16,
    f13: extern "C" fn(*mut c_void, u32, u32) -> *const u16,
    f14: extern "C" fn(*mut c_void, u32, u32) -> i32,
    f15: extern "C" fn(*mut c_void) -> u32,
    f16: extern "C" fn(*mut c_void) -> u32,
}

// C 互換構造体から呼ばれるコールバック。ctx は BonObject へのポインタ。
extern "C" fn cb_open(ctx: *mut c_void) -> i32 {
    guard(util::FALSE, || unsafe {
        tuner(ctx as *mut BonObject).open_tuner()
    })
}
extern "C" fn cb_close(ctx: *mut c_void) {
    guard((), || unsafe { tuner(ctx as *mut BonObject).close_tuner() })
}
extern "C" fn cb_set_ch_byte(ctx: *mut c_void, ch: u8) -> i32 {
    guard(util::FALSE, || unsafe {
        tuner(ctx as *mut BonObject).set_channel_byte(ch)
    })
}
extern "C" fn cb_signal(ctx: *mut c_void) -> f32 {
    guard(0.0, || unsafe {
        tuner(ctx as *mut BonObject).get_signal_level()
    })
}
extern "C" fn cb_wait(ctx: *mut c_void, timeout: u32) -> u32 {
    guard(util::WAIT_ABANDONED, || unsafe {
        tuner(ctx as *mut BonObject).wait_ts_stream(timeout)
    })
}
extern "C" fn cb_ready(ctx: *mut c_void) -> u32 {
    guard(0, || unsafe {
        tuner(ctx as *mut BonObject).get_ready_count()
    })
}
extern "C" fn cb_ts_copy(
    ctx: *mut c_void,
    pdst: *mut u8,
    psize: *mut u32,
    premain: *mut u32,
) -> i32 {
    guard(util::FALSE, || unsafe {
        tuner(ctx as *mut BonObject).get_ts_stream_copy(pdst, psize, premain)
    })
}
extern "C" fn cb_ts_ptr(
    ctx: *mut c_void,
    ppdst: *mut *mut u8,
    psize: *mut u32,
    premain: *mut u32,
) -> i32 {
    guard(util::FALSE, || unsafe {
        tuner(ctx as *mut BonObject).get_ts_stream_ptr(ppdst, psize, premain)
    })
}
extern "C" fn cb_purge(ctx: *mut c_void) {
    guard((), || unsafe {
        tuner(ctx as *mut BonObject).purge_ts_stream()
    })
}
extern "C" fn cb_release(ctx: *mut c_void) {
    guard((), || release_instance(ctx as *mut BonObject))
}
extern "C" fn cb_tuner_name(ctx: *mut c_void) -> *const u16 {
    guard(std::ptr::null(), || unsafe {
        tuner(ctx as *mut BonObject).tuner_name_ptr()
    })
}
extern "C" fn cb_is_opening(ctx: *mut c_void) -> i32 {
    guard(util::FALSE, || unsafe {
        tuner(ctx as *mut BonObject).is_tuner_opening()
    })
}
extern "C" fn cb_enum_space(ctx: *mut c_void, space: u32) -> *const u16 {
    guard(std::ptr::null(), || unsafe {
        tuner(ctx as *mut BonObject).enum_tuning_space(space)
    })
}
extern "C" fn cb_enum_ch(ctx: *mut c_void, space: u32, channel: u32) -> *const u16 {
    guard(std::ptr::null(), || unsafe {
        tuner(ctx as *mut BonObject).enum_channel_name(space, channel)
    })
}
extern "C" fn cb_set_ch2(ctx: *mut c_void, space: u32, channel: u32) -> i32 {
    guard(util::FALSE, || unsafe {
        tuner(ctx as *mut BonObject).set_channel(space, channel)
    })
}
extern "C" fn cb_cur_space(ctx: *mut c_void) -> u32 {
    guard(0, || unsafe {
        tuner(ctx as *mut BonObject).get_cur_space()
    })
}
extern "C" fn cb_cur_ch(ctx: *mut c_void) -> u32 {
    guard(0, || unsafe {
        tuner(ctx as *mut BonObject).get_cur_channel()
    })
}

/// BonObject を C 互換構造体でラップして返す。
///
/// EDCB は `pEnd - base >= sizeof(STRUCT_IBONDRIVER2)` で IBonDriver2 対応を
/// 判定するため、`pEnd` は構造体の直後を指すようにする。
fn build_bon_struct(obj: *mut BonObject) -> *const c_void {
    unsafe {
        // 既に構築済みならそれを返す。
        if !(*obj).holder.is_null() {
            return (*obj).holder as *const c_void;
        }
        let holder = Box::new(BonStruct2 {
            base: BonStruct1 {
                ctx: obj as *mut c_void,
                end: std::ptr::null(),
                f00: cb_open,
                f01: cb_close,
                f02: cb_set_ch_byte,
                f03: cb_signal,
                f04: cb_wait,
                f05: cb_ready,
                f06: cb_ts_copy,
                f07: cb_ts_ptr,
                f08: cb_purge,
                f09: cb_release,
            },
            f10: cb_tuner_name,
            f11: cb_is_opening,
            f12: cb_enum_space,
            f13: cb_enum_ch,
            f14: cb_set_ch2,
            f15: cb_cur_space,
            f16: cb_cur_ch,
        });
        let p = Box::into_raw(holder);
        let end = (p as *const u8).add(std::mem::size_of::<BonStruct2>());
        (*p).base.end = end as *const c_void;
        (*obj).holder = p as *mut c_void;
        p as *const c_void
    }
}

/// コンパイラ非依存の C 互換構造体を返す(Linux の EDCB 等が優先して使う)。
#[unsafe(no_mangle)]
pub extern "C" fn CreateBonStruct() -> *const c_void {
    guard(std::ptr::null(), || {
        let _lock = INSTANCE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let obj = create_instance_locked();
        build_bon_struct(obj)
    })
}

// ---- DLL エントリポイント (Windows のみ) -------------------------------

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::Foundation::{BOOL, HINSTANCE, MAX_PATH};
    use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;

    const DLL_PROCESS_ATTACH: u32 = 1;
    const DLL_PROCESS_DETACH: u32 = 0;

    #[unsafe(no_mangle)]
    pub extern "system" fn DllMain(hinst: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
        match reason {
            DLL_PROCESS_ATTACH => {
                HMODULE_VALUE.store(hinst as isize, Ordering::SeqCst);
                // INIを読み込む。失敗したらロードを拒否する(元実装と同じ)。
                match load_config(hinst) {
                    Some(cfg) => {
                        let _ = CONFIG.set(cfg);
                        util::TRUE
                    }
                    None => util::FALSE,
                }
            }
            DLL_PROCESS_DETACH => {
                // 未開放インスタンスがあれば開放する。
                let inst = INSTANCE.load(Ordering::SeqCst);
                if !inst.is_null() {
                    release_instance(inst);
                }
                util::TRUE
            }
            _ => util::TRUE,
        }
    }

    /// DLLと同じ場所にある `<dll名>.ini` を読み込む。
    fn load_config(hinst: HINSTANCE) -> Option<Config> {
        let mut buf = vec![0u16; MAX_PATH as usize];
        let len = unsafe { GetModuleFileNameW(hinst as _, buf.as_mut_ptr(), buf.len() as u32) };
        if len == 0 {
            return None;
        }
        let dll_path = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
        let ini_path = dll_path.with_extension("ini");
        Config::load(&ini_path)
    }
}

/// 非 Windows 向けの初期化フック。共有ライブラリのロード時に INI を読む。
///
/// `dladdr` で自身の共有ライブラリパスを取得し、次の順に設定ファイルを探索する
/// (BonDriver_LinuxMirakc と同様に `.so` の隣に置いた INI を読む):
///   1. 環境変数 `BONDRIVER_MIRAKURUN_INI` が指すファイル
///   2. `<ライブラリパス>.ini`(例: `/usr/local/lib/edcb/BonDriver_Mirakurun.so.ini`)
///   3. `<ライブラリパスから拡張子を除いたもの>.ini`(例: `.../BonDriver_Mirakurun.ini`)
///   4. カレントディレクトリの `BonDriver_Mirakurun.ini`
///
/// いずれも見つからない場合は既定値で動作する。
#[cfg(not(windows))]
extern "C" fn init_config() {
    let cfg = config_paths()
        .iter()
        .find_map(|p| Config::load(p))
        .unwrap_or_else(|| Config::parse(""));
    let _ = CONFIG.set(cfg);
}

/// Linux の dladdr(3) 互換構造体。
#[cfg(not(windows))]
#[repr(C)]
struct DlInfo {
    dli_fname: *const std::ffi::c_char,
    dli_fbase: *mut c_void,
    dli_sname: *const std::ffi::c_char,
    dli_saddr: *mut c_void,
}

#[cfg(not(windows))]
unsafe extern "C" {
    fn dladdr(addr: *const c_void, info: *mut DlInfo) -> std::ffi::c_int;
}

/// 自身の共有ライブラリの絶対パスを返す(取得できなければ None)。
#[cfg(not(windows))]
fn self_library_path() -> Option<PathBuf> {
    let mut info: DlInfo = unsafe { std::mem::zeroed() };
    // このモジュール内の関数アドレスを渡すと自身のライブラリ情報が得られる。
    let addr = init_config as *const c_void;
    let rc = unsafe { dladdr(addr, &mut info) };
    if rc == 0 || info.dli_fname.is_null() {
        return None;
    }
    unsafe {
        std::ffi::CStr::from_ptr(info.dli_fname)
            .to_str()
            .ok()
            .map(PathBuf::from)
    }
}

/// INI 候補パスを優先順に返す(非 Windows)。
#[cfg(not(windows))]
fn config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(p) = std::env::var("BONDRIVER_MIRAKURUN_INI")
        && !p.is_empty()
    {
        paths.push(PathBuf::from(p));
    }

    if let Some(lib) = self_library_path() {
        // `.../BonDriver_Mirakurun.so.ini` と `.../BonDriver_Mirakurun.ini` の両方。
        let mut file_name = lib
            .file_name()
            .map(|s| s.to_os_string())
            .unwrap_or_default();
        file_name.push(".ini");
        paths.push(lib.with_file_name(file_name));
        paths.push(lib.with_extension("ini"));
    }

    paths.push(PathBuf::from("BonDriver_Mirakurun.ini"));
    paths
}

/// 共有ライブラリのコンストラクタ。INI を読み込む(非 Windows)。
#[cfg(not(windows))]
#[used]
#[unsafe(link_section = ".init_array")]
static INIT: extern "C" fn() = init_config;
