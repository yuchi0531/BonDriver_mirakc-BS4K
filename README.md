# BonDriver_Mirakurun (Rust)

[Mirakurun](https://github.com/Chinachu/Mirakurun) / [mirakc](https://github.com/mirakc/mirakc)
(mirakc-BS4K フォーク含む)を利用する為の BonDriver です。

本リポジトリはオリジナルの C++ 実装を **Rust に全面移植** したもので、
**Windows(TVTest 等)と Linux(EDCB の Unix 版)の両方**で動作します。
単一のコードベースから次のターゲットをビルドできます。

| ターゲット | 生成物 | 主な用途 |
| --- | --- | --- |
| `x86_64-pc-windows-msvc` | `BonDriver_Mirakurun.dll` | TVTest (64bit) |
| `i686-pc-windows-msvc` | `BonDriver_Mirakurun.dll` | TVTest (32bit) |
| `x86_64-unknown-linux-gnu` | `BonDriver_Mirakurun.so` | EDCB (Linux) |

対応する独自機能:

- **BS4K(MMT/TLV)パススルー**: `type=BS4K` のチャンネルは内容に依存せず
  バイト列のまま中継します(mirakc 側で `tlv-decoder` 未設定なら TLV のまま)。
- **チューナー固定(`X-Mirakc-Tuner`)**: channel stream 取得時にチューナー名を
  指定して厳密に固定できます(後述)。

## 必要環境

- [rustup](https://rustup.rs/)(ツールチェーン管理)
- 対応アーキテクチャ: Windows **x64 / x86(32bit)**、Linux **x64**
  - Windows x86 ではメンバ関数が `__thiscall`(this を ECX で渡す)規約に
    なるため、vtable スロットを `extern "thiscall"` で構築しています。

### Windows

- リンカとして次のいずれか
  - **MSVC Build Tools**(既定。C++ ワークロード + Windows SDK。`x86_64-pc-windows-msvc` 用)
  - もしくは **LLVM-MinGW**(`x86_64-pc-windows-gnu` 用)

MSVC ビルド環境を一括で用意したい場合は、付属の `setup-env.bat` を実行してください。
[winget configure](https://learn.microsoft.com/windows/package-manager/configuration/)
を利用して `configuration.dsc.yaml` に定義された以下をインストールします。

- Visual Studio 2022 Build Tools(VCTools ワークロード + Windows 11 SDK)
- Rustup(Rust ツールチェーン管理)

```bat
setup-env.bat
```

### Linux

- `rustup` で `x86_64-unknown-linux-gnu` を用意(通常のインストールで可)。
- 追加の C ライブラリは不要です(`dladdr(3)` は libc に含まれます)。

## ビルド

### Windows

付属の `build.bat` を実行してください。

```bat
build.bat
```

- 既定ターゲットは `x86_64-pc-windows-msvc`(MSVC)です。
- 生成物は `dist\BonDriver_Mirakurun.dll` と `dist\BonDriver_Mirakurun.ini` に出力されます。
- 32bit 版が必要な場合は引数で渡せます:

```bat
build.bat i686-pc-windows-msvc
```

- 別ターゲットを指定する場合は引数で渡せます。LLVM-MinGW 環境なら:

```bat
build.bat x86_64-pc-windows-gnu
```

### Linux

付属の `build.sh` を実行してください。

```sh
./build.sh
```

- 既定ターゲットは `x86_64-unknown-linux-gnu` です。
- 生成物は `dist/BonDriver_Mirakurun.so` と `dist/BonDriver_Mirakurun.so.ini` に出力されます。

手動でビルドする場合:

```sh
cargo build --release
cp target/release/libBonDriver_Mirakurun.so dist/BonDriver_Mirakurun.so
```

### 3ターゲット一括ビルド(リリース zip)

`release.sh` を使うと、win-x86_64 / win-x86 / linux-x86_64 をまとめて
ビルドし、配布用 zip を `release/` に出力します。

```sh
./release.sh            # バージョンは Cargo.toml から取得
./release.sh 2.2.0      # バージョン明示
```

出力:

```
release/BonDriver_Mirakurun-<ver>-win-x86_64.zip   (DLL + ini)
release/BonDriver_Mirakurun-<ver>-win-x86.zip      (DLL + ini)
release/BonDriver_Mirakurun-<ver>-linux-x86_64.zip (so + so.ini)
```

- Windows ターゲットのクロスコンパイルには `cargo-xwin` が必要です:
  `cargo install cargo-xwin`
- cargo-xwin 同梱の lld-link は `/export:...,EXPORTAS,...` を解釈できないため、
  `release.sh` は rustup 同梱の新しい `lld-link`(rust-lld)を使うラッパを
  自動生成します。別のリンカを使う場合は `LLD_LINK=/path/to/lld-link ./release.sh`。

## インストール

### Windows(TVTest)

`dist\BonDriver_Mirakurun.dll` を TVTest の `BonDriver` フォルダ(または BonDriver
検索パス)に置きます。同じフォルダの `BonDriver_Mirakurun.ini` が読み込まれます。

### Linux(EDCB)

EDCB の BonDriver ディレクトリ(既定 `/usr/local/lib/edcb/`)に配置します。

```sh
sudo cp dist/BonDriver_Mirakurun.so     /usr/local/lib/edcb/
sudo cp dist/BonDriver_Mirakurun.so.ini /usr/local/lib/edcb/
```

- INI は **`<so名>.ini`**(例: `BonDriver_Mirakurun.so.ini`)として `.so` と同じ
  ディレクトリに置きます(`BonDriver_LinuxMirakc` と同じ流儀です)。
  拡張子を除いた `BonDriver_Mirakurun.ini` も探索します。
- GR 用と BS/CS 用でチューナーを分ける場合は、同じ `.so` を
  `BonDriver_Mirakurun_T.so` / `BonDriver_Mirakurun_S.so` のように複製し、
  それぞれに対応する `.so.ini` を用意します。
  EDCB の `EpgTimerSrv.ini` で各セクションの `Priority` を **重複しない値** に
  してください(重複すると片方が認識されません)。

## 設定

DLL/SO と同じ場所の INI を読み込みます。項目はオリジナル版と互換で、
すべて `[GLOBAL]` セクションに記述します。

| キー | 既定値 | 説明 |
| --- | --- | --- |
| `SERVER_HOST` | `localhost` | Mirakurun/mirakc のホスト名(`http` 時) |
| `SERVER_PORT` | `40772` | Mirakurun/mirakc のポート番号(`BonDriver_mirakc` と同一の既定値) |
| `SERVER_TYPE` | `http` | `http` または `unix`(Linux のみ) |
| `SERVER_SOCKPATH` | (空) | `SERVER_TYPE=unix` のときの Unix ソケットパス |
| `DECODE_B25` | `0` | B25 デコード(1=有効)。ストリーム URL の `decode=` に反映 |
| `PRIORITY` | `0` | Mirakurun の優先度(`X-Mirakurun-Priority` ヘッダ。一覧取得/ストリーム双方で送信) |
| `SERVICE_SPLIT` | `0` | サービス単位で分割(1=有効) |
| `TUNER` | (空) | チューナー固定名(`X-Mirakc-Tuner` ヘッダ)。空欄で無効 |

`SERVER_TYPE="unix"` の設定例(接続方式以外は同じ):

```ini
[GLOBAL]
SERVER_TYPE="unix"
SERVER_SOCKPATH="/var/run/mirakc.sock"
```

## BonDriver_mirakc との互換

本ドライバは mirakc が Windows 用に推奨している
[`tkmsst/BonDriver_mirakc`](https://github.com/tkmsst/BonDriver_mirakc)
(mirakc 作者による `BonDriver_Mirakurun` フォーク)の挙動に合わせています。

- **エンドポイント**:
  - 一覧: `SERVICE_SPLIT=0` → `GET /api/channels`、`SERVICE_SPLIT=1` → `GET /api/services`
  - ストリーム: `SERVICE_SPLIT=0` → `GET /api/channels/{type}/{channel}/stream?decode={0|1}`、
    `SERVICE_SPLIT=1` → `GET /api/services/{id}/stream?decode={0|1}`
    (`{id}` はサービス一覧の `id` = NID*100000+SID)
- **ヘッダ**: `Connection: close` と `X-Mirakurun-Priority: <PRIORITY>` を
  一覧取得・ストリーム取得の両方で送信。加えて `User-Agent` を送出。
- **リトライ**: ストリーム取得が非 200(チューナー使用中・不在など)の場合、
  500ms 空けて **1 回だけ**再試行します。
- **選局の完了扱い**: 再試行しても取得できない場合でも `SetChannel` は
  `TRUE` を返します(`BonDriver_mirakc` と同じ。`GetReadyCount` は 0 のまま)。
- **ビットレート**: `GetSignalLevel` は受信バイト量から算出した Mbps で、
  100Mbps 上限、計測窓は 1000ms です。
- IP アドレス/ホスト名の解決は `ToSocketAddrs` により IPv4/IPv6/ホスト名すべて対応
  (`BonDriver_mirakc` の WinHTTP 相当)。

## チューナー固定(`X-Mirakc-Tuner`)

mirakc(mirakc-BS4K)は `GET /api/channels/{type}/{channel}/stream` に対して
`X-Mirakc-Tuner: <チューナー名>` ヘッダを受け付け、該当名のチューナーを
**厳密に**使用します(他のチューナーへのフォールバックはしません)。

- 固定したい場合は INI の `TUNER` に `config.tuners[].name` と同じ文字列を設定します。
  設定すると、channel stream 取得時に自動でヘッダが付与されます。
- **対応放送種別による絞り込み**: チューナーを固定すると、`/api/tuners` を参照して
  そのチューナーが対応する放送種別(`types`。GR/BS/CS/SKY/BS4K/CATV)だけを
  チャンネル一覧に表示します。衛星専用チューナーを指定したときに地上波が、
  地上波専用チューナーを指定したときに衛星が一覧に出て選局時に HTTP 404 に
  なる問題を防ぎます。`/api/tuners` が取得できない場合や `types` が空の場合は
  絞り込まず全波を表示します(従来動作)。
- API の `/api/channels` 応答に `tuner` フィールドがある場合は、そちらが
  チャンネル単位で優先されます。
- このヘッダは **channel stream(GET)** でのみ有効です。`/api/services/{id}/stream`
  や `/api/channels/.../services/{sid}/stream`(`SERVICE_SPLIT=1` 時)では無視されます。
- 固定先が見つからない場合は HTTP 400、利用不可の場合は HTTP 503 となり、
  `SetChannel` は失敗します(`OpenTuner` 成功後もストリームが得られません)。

## 実装メモ

- **ABI(Windows x64/x86)**: `CreateBonDriver` は MSVC が実際に生成する vtable
  スロット順(同名オーバーロードの逆順配置、`IBonDriver2` で再宣言される
  `Release` の基底スロット共有を含む全17スロット)に一致させています。
  - x86(32bit)ではメンバ関数が `__thiscall`(this を ECX、スタックは呼ばれた側が
    掃除)のため、vtable スロットを `extern "thiscall"` で構築しています。
  - TVTest(LibISDB)は `dynamic_cast<IBonDriver2*>` を実行するため、MSVC 互換の
    RTTI 構造体を実行時に構築し vtable に付加しています(`src/rtti.rs`)。
    x64 は image-relative 参照(COL 24byte)、x86 は絶対アドレス参照
    (COL 20byte)で構築します。
- **ABI(Linux)**: ホスト(EDCB)は `CreateBonStruct`(コンパイラ非依存の C 互換
  構造体)を優先して使うため、Linux では `dynamic_cast` 不要です。本実装は
  `CreateBonStruct` と `CreateBonDriver`(Itanium の宣言順 vtable)の両方を
  エクスポートします。
- **トランスポート**: TCP(`SERVER_TYPE=http`)と Unix ドメインソケット
  (`SERVER_TYPE=unix`)の両対応。TCP はホスト名/IPv4/IPv6 を `ToSocketAddrs` で解決。
- **TS 受信**: オリジナルの Push/Pop 2スレッド + オーバーラップド I/O を、
  1本の受信スレッド + `Mutex`/`Condvar` キューという等価構成へ整理しています。
  ストリーム先頭の HTTP レスポンスヘッダは TS/TLV データに混入させず読み飛ばします。
- **BS4K(TLV)**: ペイロードの内容に依存せずバイト列をそのまま中継します。
  サーバ側で `tlv-decoder` が設定されていれば通常の MPEG-TS として届きます。
- **JSON 解析**: `serde_json` を使用(オリジナルの picojson 置き換え)。
- **INI 解決(Windows)**: `GetModuleFileNameW` で DLL 自身のパスを取得し
  `<dll名>.ini` を読みます。読込に失敗した場合はロードを拒否します。
- **INI 解決(Linux)**: `dladdr(3)` で `.so` 自身のパスを取得し
  `<so名>.ini` を読みます。見つからない場合は既定値で動作します。
  環境変数 `BONDRIVER_MIRAKURUN_INI` で明示指定も可能です。
- **panic 安全**: FFI(`extern "C"`)境界を Rust の unwind が越えないよう、
  リリースビルドでは `panic = "abort"` を指定しています。

## テスト

```sh
cargo test          # プラットフォーム非依存ロジックのユニットテスト
```

## License

This software is released under the MIT License, see LICENSE.
