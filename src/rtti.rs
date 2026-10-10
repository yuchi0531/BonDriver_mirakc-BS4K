//! MSVC RTTI エミュレーション(x64 / x86)。
//!
//! TVTest(LibISDB)は CreateBonDriver の戻り値に対して
//! `dynamic_cast<IBonDriver2*>` を実行する。MSVC の dynamic_cast は
//! vtable の直前(`vtable[-1]`)に置かれた RTTI Complete Object Locator を
//! 辿るため、これを持たない vtable を渡すとゴミポインタの参照から
//! 未捕捉例外となり、ホストが 0xC0000409 で異常終了する。
//!
//! ここでは vtable と MSVC 互換の RTTI 構造体一式を1つの連続ブロックに
//! 実行時構築する。ポインタ表現はアーキテクチャで異なる:
//!
//! - **x64**: vtable[-1] は COL への 8byte 絶対ポインタ。COL 内部の参照
//!   (pTypeDescriptor / pClassDescriptor / pSelf)は「モジュールベースからの
//!   32bit オフセット(image-relative)」。ベースアドレスは
//!   `COLのアドレス - COL.pSelf` で逆算される仕様のため、ブロック先頭を
//!   疑似ベースとすれば実行時確保のメモリでも整合する。COL は 24 バイト。
//! - **x86**: vtable[-1] は COL への 4byte 絶対ポインタ。COL 内部の参照も
//!   すべて絶対アドレス(32bit VA)。`pSelf` は無く COL は 20 バイト。
//!
//! RTTI 内部の参照フィールド・BaseClassArray の要素は両アーキテクチャとも
//! 32bit(4byte)である。異なるのは「絶対 / 相対」と COL の有無、
//! vtable スロット幅、TypeDescriptor ヘッダ幅のみ。
//!
//! 型の比較は TypeDescriptor のマングル名文字列(strcmp)で行われるので、
//! ホスト側の `typeid(IBonDriver2)` と名前が一致すればキャストは成功する。

use std::alloc::{Layout, alloc_zeroed};

/// vtable スロット / vtable[-1] / TypeDescriptor 先頭ポインタの幅。
const PTR: usize = std::mem::size_of::<usize>();
/// RTTI 内部参照フィールド・BaseClassArray 要素の幅(常に 32bit)。
const RPTR: usize = 4;

const VTABLE_SLOTS: usize = 17;
/// vtable[-1] スロットを挟んだ vtable 本体のオフセット。
const VTBL_OFF: usize = PTR;
const AFTER_VTBL: usize = VTBL_OFF + VTABLE_SLOTS * PTR;

/// RTTICompleteObjectLocator のサイズ。x64 は pSelf を含む 24 バイト。
#[cfg(target_pointer_width = "64")]
const COL_SIZE: usize = 24;
#[cfg(target_pointer_width = "32")]
const COL_SIZE: usize = 20;

const CHD_SIZE: usize = 16; // signature, attributes, numBaseClasses, pBCA
const BCD_SIZE: usize = 28; // pTD, numContained, mdisp, pdisp, vdisp, attributes, pCHD
/// TypeDescriptor 先頭のポインタ2つ(vftable, spare)のサイズ。
const TD_HEADER: usize = 2 * PTR;

/// ホスト側 typeid() のマングル名と厳密一致させること。
const TD2_NAME: &[u8] = b".?AVIBonDriver2@@\0";
const TD1_NAME: &[u8] = b".?AVIBonDriver@@\0";

/// 4byte 境界に切り上げる。
const fn al(n: usize) -> usize {
    n.div_ceil(RPTR) * RPTR
}

// ---- ブロックレイアウト(x86/x64 双方で自己整合)-------------------------
const COL_OFF: usize = al(AFTER_VTBL);
const CHD_OFF: usize = COL_OFF + al(COL_SIZE);
const BCA_OFF: usize = CHD_OFF + al(CHD_SIZE);
const BCD0_OFF: usize = BCA_OFF + al(2 * RPTR); // [IBonDriver2, IBonDriver]
const BCD1_OFF: usize = BCD0_OFF + al(BCD_SIZE);
const CHD1_OFF: usize = BCD1_OFF + al(BCD_SIZE);
const BCA1_OFF: usize = CHD1_OFF + al(CHD_SIZE);
const TD2_OFF: usize = BCA1_OFF + al(1 * RPTR); // [IBonDriver]
const TD1_OFF: usize = TD2_OFF + al(TD_HEADER + TD2_NAME.len());
const BLOCK_SIZE: usize = TD1_OFF + al(TD_HEADER + TD1_NAME.len());

// レイアウトの重複・ブロック外書き込みをコンパイル時に検出する。
const _: () = {
    assert!(VTBL_OFF + VTABLE_SLOTS * PTR <= COL_OFF);
    assert!(COL_OFF + COL_SIZE <= CHD_OFF);
    assert!(TD2_OFF + TD_HEADER + TD2_NAME.len() <= TD1_OFF);
    assert!(TD1_OFF + TD_HEADER + TD1_NAME.len() <= BLOCK_SIZE);
};

unsafe fn put_u32(base: *mut u8, off: usize, v: u32) {
    unsafe { (base.add(off) as *mut u32).write(v) };
}

#[cfg(target_pointer_width = "64")]
unsafe fn put_u64(base: *mut u8, off: usize, v: u64) {
    unsafe { (base.add(off) as *mut u64).write(v) };
}

unsafe fn put_bytes(base: *mut u8, off: usize, v: &[u8]) {
    unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), base.add(off), v.len()) };
}

/// RTTI 内部参照を書く。x64 はブロック先頭からの相対オフセット、
/// x86 は絶対アドレス。
#[inline]
unsafe fn put_ref(base: *mut u8, off: usize, target_off: usize) {
    #[cfg(target_pointer_width = "64")]
    unsafe {
        put_u32(base, off, target_off as u32);
    }
    #[cfg(target_pointer_width = "32")]
    unsafe {
        put_u32(base, off, (base as usize + target_off) as u32);
    }
}

/// vtable 本体(サイズ `vtbl_size`)を RTTI 付きブロックへ複製し、
/// ホストへ渡せる vtable ポインタ(ブロック内 `VTBL_OFF`)を返す。
///
/// 返したメモリは DLL の寿命中ずっと使われるため意図的に解放しない。
pub unsafe fn build_vtable_with_rtti(vtbl_src: *const u8, vtbl_size: usize) -> *const u8 {
    assert!(vtbl_size <= VTABLE_SLOTS * PTR);

    let layout = Layout::from_size_align(BLOCK_SIZE, PTR).unwrap();
    let base = unsafe { alloc_zeroed(layout) };
    assert!(!base.is_null());

    unsafe {
        // vtable 本体を複製。
        std::ptr::copy_nonoverlapping(vtbl_src, base.add(VTBL_OFF), vtbl_size);

        // vtable[-1]: COL への絶対ポインタ(x64=8byte, x86=4byte)。
        let col_addr = base as usize + COL_OFF;
        #[cfg(target_pointer_width = "64")]
        put_u64(base, 0, col_addr as u64);
        #[cfg(target_pointer_width = "32")]
        put_u32(base, 0, col_addr as u32);

        // COL。x64 は signature=1(image-relative) + pSelf、x86 は signature=0(絶対)。
        put_u32(
            base,
            COL_OFF,
            if cfg!(target_pointer_width = "64") {
                1
            } else {
                0
            },
        );
        put_u32(base, COL_OFF + 4, 0); // vtable の完全体内オフセット
        put_u32(base, COL_OFF + 8, 0); // constructor displacement
        put_ref(base, COL_OFF + 12, TD2_OFF); // pTypeDescriptor
        put_ref(base, COL_OFF + 16, CHD_OFF); // pClassDescriptor
        #[cfg(target_pointer_width = "64")]
        put_u32(base, COL_OFF + 20, COL_OFF as u32); // pSelf

        // 完全型 IBonDriver2 の階層: attributes=0 (単一継承・非仮想継承)。
        put_u32(base, CHD_OFF, 0);
        put_u32(base, CHD_OFF + 4, 0);
        put_u32(base, CHD_OFF + 8, 2); // 自身 + IBonDriver
        put_ref(base, CHD_OFF + 12, BCA_OFF);
        put_ref(base, BCA_OFF, BCD0_OFF);
        put_ref(base, BCA_OFF + RPTR, BCD1_OFF);

        // RTTIBaseClassDescriptor を書くヘルパ。
        // PMD = {mdisp:0, pdisp:-1, vdisp:0}(オフセット0・仮想基底なし)、
        // attributes=0x40 は pClassDescriptor フィールド有効の印。
        unsafe fn write_bcd(base: *mut u8, off: usize, td: usize, num_contained: u32, chd: usize) {
            unsafe {
                put_ref(base, off, td);
                put_u32(base, off + 4, num_contained);
                put_u32(base, off + 8, 0);
                put_u32(base, off + 12, -1i32 as u32);
                put_u32(base, off + 16, 0);
                put_u32(base, off + 20, 0x40);
                put_ref(base, off + 24, chd);
            }
        }
        // IBonDriver2(自身) / IBonDriver
        write_bcd(base, BCD0_OFF, TD2_OFF, 1, CHD_OFF);
        write_bcd(base, BCD1_OFF, TD1_OFF, 0, CHD1_OFF);

        // IBonDriver 単体の階層(BCD1.pClassDescriptor の参照先)。
        put_u32(base, CHD1_OFF, 0);
        put_u32(base, CHD1_OFF + 4, 0);
        put_u32(base, CHD1_OFF + 8, 1);
        put_ref(base, CHD1_OFF + 12, BCA1_OFF);
        put_ref(base, BCA1_OFF, BCD1_OFF);

        // TypeDescriptor: 先頭の type_info vftable ポインタは dynamic_cast では
        // 参照されないため 0 のままにする。名前はマングル名で厳密一致させる。
        put_bytes(base, TD2_OFF + TD_HEADER, TD2_NAME);
        put_bytes(base, TD1_OFF + TD_HEADER, TD1_NAME);

        base.add(VTBL_OFF)
    }
}
