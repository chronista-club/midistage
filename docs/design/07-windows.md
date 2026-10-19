# 07. Windows 版 midistaged

> **Status**: Draft
> **Related**: design 03（共通サービス）、design 05（常駐と配布）、design 06（ROTO の setup）、実測 `mem_1CgBB8Xpu4BLF1v1hDFtAY`（2026-10-19）
> **対象**: `crates/midistaged`（`native`・`settings`・`paths`・`main`・`roto_admin`）、`crates/midistage-core`、配布

## Abstract

Windows 11 でも midistaged を動かし、アプリ A／B が同じ MIDI コントローラーを lease で引き継げるようにする。Windows 11 の新しい MIDI スタック（Windows MIDI Services、サービス `midisrv`）は、複数のアプリが同じ機材を開くことをすでに許す。このため Windows でも、macOS の CoreMIDI と同じく「全員が開けるが、使用権は誰も調停しない」状況になる。midistage の契約（design 03）は変えない。OS 境界（MIDI の I/O、ファイルの排他、置き場、常駐、Serial）だけを差し替える。

## 前提（実測 2026-10-19、Windows 11 Pro 10.0.26200、nanoKONTROL2）

- `midisrv` は OS に組み込みで入っていた。nanoKONTROL2 は KORG ドライバ無しで Microsoft の `usbmidi2.inf` に乗り、`midisrv` の endpoint になる
- 旧 API（WinMM）でも、2 つのプロセスが同時に In / Out を開ける（`MMSYSERR_ALLOCATED` にならない）。入力は両方に同じ件数だけ届く（60 秒で 625 件ずつ）
- SysEx は 1 メッセージ単位で通る。2 つのプロセスが同時に要求を送っても、402 byte のダンプ応答に別の応答が混ざることはなかった（40/40）
- 応答は全 client に配られる。誰の要求への応答か、アプリの側では区別できない
- 要求を間隔ゼロで連打すると、1 プロセスだけでも機材側で落ちる（200 中 184）。10 ms 空けると落ちない
- 未確認: 長い SysEx を**書き込む**ときの原子性（host → 機材）、KORG USB-MIDI Driver を入れた場合の挙動

OS が解いたのは「開ける・壊れない」まで。要求の順番待ち、要求と応答の対応、使用権、引き継ぎの完了待ちは、macOS と同じく midistaged の仕事として残る。

## 決めたこと

### 対象は Windows MIDI Services のある Windows 11 だけ

`midisrv` の無い旧スタックは対象外。旧スタックでは物理ポートが 1 アプリ排他になる。そのため、物理ポートは midistaged が独占し、アプリには仮想ポートを配る、という形になる。ところが旧スタックには OS 標準の仮想ポートが無く、loopMIDI などの外部ドライバが必要になる。配布と保守の負担に見合わない。

### 物理ポートを開くのはサービスだけ、は約束のまま

`midisrv` は誰にでも開かせるので、OS はこの約束を強制しない。macOS の CoreMIDI と同じで、アプリ側の約束（自分の bridge ポート以外を選ばない）として扱う。design 03 の不変条件は変えない。

### MIDI の I/O は Windows MIDI Services の SDK で書く

`native_midi` 拡張には、lease ごとにアプリ専用の仮想 source / destination が要る（design 03）。WinMM には仮想ポートを作る API が無いので、仮想 endpoint を作れる Windows MIDI Services の App SDK（WinRT、`Microsoft.Windows.Devices.Midi2`）を使う。UMP がそのまま通るので、`midistage-core` の UMP 型とも合う。

Rust からは、SDK の winmd から `windows-bindgen` で binding を作る見込み。`windows` crate に入っている `Windows.Devices.Midi` は旧来の MIDI 1.0 用 API で、仮想 endpoint を作れないので使わない。

### `native` を OS 共通部分と backend に分ける

`native.rs` は今、`cfg(target_os = "macos")` の 1 ファイルに、CoreMIDI の呼び出しと、OS に依らない段取り（lease の世代で送信を閉じる、`io_fence` で完了を待つ、finalizer を送る）が一緒に入っている。

```
crates/midistaged/src/native/
  mod.rs        # OS 共通: NativeRuntime の poll、bridge の開閉の段取り、output_worker、finalizer
  coremidi.rs   # cfg(macos): 列挙・物理 In/Out・仮想 source/destination・SysEx completion・抜き差し通知
  winmidi.rs    # cfg(windows): 同じ面を Windows MIDI Services で
```

backend が満たす面は次の 5 つ。trait にするか、cfg で同名の関数を出し分けるかは、分けたときのコードの形を見て決める。

1. 列挙（read-only）: 物理機材ごとのポート一覧を `inventory::DevicePorts` に入れる。Windows では endpoint device ID と、親の USB デバイス（VID / PID / instance）で、一台の機材のポートをまとめる
2. 物理 In の購読（timestamp 付き）
3. 物理 Out への送信と、その完了の通知
4. lease ごとの仮想 source / destination の作成と破棄
5. 抜き差しの通知

### 「送信の完了」の意味を Windows で確かめてから実装する

design 03 の引き継ぎは、送信中の SysEx の完了を確認するまで次の持ち主を active にしない。CoreMIDI では `MIDISendSysex` の completion がこの完了にあたる。Windows MIDI Services の送信 API が戻った時点が、`midisrv` に渡しただけの時点なのか、機材に出た時点なのかは、まだ分かっていない。

前者なら、`midisrv` への受け渡しをもって完了とし、機材が受け付けるまでの余白を別に持つ。macOS で仮想 destination の completion を実機の完了として扱わないのと同じ区別を、design にも書く。タイムアウトだけで先へ進めない、という不変条件は変えない。

### OS 境界の小物

| 箇所 | macOS（今） | Windows |
|---|---|---|
| `settings.rs` の排他 | `flock(LOCK_EX \| LOCK_NB)` | `LockFileEx`（`LOCKFILE_EXCLUSIVE_LOCK \| LOCKFILE_FAIL_IMMEDIATELY`） |
| `settings.rs` の権限 | `0600` / `0700` | `%LOCALAPPDATA%` 配下に置き、ユーザーごとの ACL に任せる（追加の ACL は付けない） |
| `paths.rs` の状態 | `~/Library/Application Support/Midistage` | `%LOCALAPPDATA%\Midistage` |
| `paths.rs` のログ | `~/Library/Logs/midistaged` | `%LOCALAPPDATA%\Midistage\logs` |
| 終了の合図 | SIGTERM + Ctrl-C | Ctrl-C + コンソール / セッションの終了イベント |
| Serial（ROTO） | `/dev/cu.*` + `O_EXLOCK` + IOKit | `\\.\COMn`（Windows では最初から排他で開く）+ SetupAPI で VID / PID から探す |

`paths.rs` と `settings.rs` は純粋な部分（パスの組み立て、一時ファイル → fsync → rename の手順）と、OS の呼び出しを分けたまま、cfg で出し分ける。

### 常駐はユーザーごとのプロセス（Windows サービスにはしない）

Windows サービスは session 0 で動く。そこからユーザーの MIDI 環境と user-private な `endpoint.json` を扱うのは筋が悪い。macOS の LaunchAgent に相当するのは、ログイン時にユーザーの権限で起動するプロセスである。タスクトレイの app が登録と解除を行う。登録の手段（スタートアップのタスク、Run キー、MSIX の StartupTask）は、配布の形（MSIX にするか）と一緒に決める。

## 実装の順番

1. **Windows で `serve` が起動する**: `settings` / `paths` / 終了の合図を出し分け、`native` 抜きで lease と状態だけ動かす。Rust SDK の相互テストを Windows で通す。`midistage-core` の CoreMIDI FFI も cfg で閉じる
2. **`list-ports`**: Windows MIDI Services で物理ポートを read-only で列挙する（何も開かず、何も送らない）。nanoKONTROL2 と Zenith 2 で確かめる
3. **native 本体**: 物理 In / Out、仮想 endpoint、送信完了の意味の確認、引き継ぎ。nanoKONTROL2 から（design 03「観測と移行」と同じ順）
4. **常駐と配布**: タスクトレイ app、署名、登録
5. **ROTO の Serial**（design 06 の Windows 版）

1 は今のコードの延長でできる。2 から先は、Windows MIDI Services の SDK を Rust から呼べることを先に確かめる（spike）。

## 未決

- SDK の runtime がアプリと一緒に配る必要のあるものか、OS に入っているものか（このマシンには `midi.exe` などの SDK ツールが入っていなかった）
- Rust binding の作り方（`windows-bindgen` で winmd から作るか、C++/WinRT の薄い shim を挟むか）
- 送信 API が戻った時点の意味（上記）
- KORG USB-MIDI Driver を入れた機材が、`midisrv` からどう見えるか（Zenith 2 は `MIDIU_KSA_…` の経路だった）
- Windows のアプリ向けの SDK（Rust SDK は使える。Swift SDK は macOS 専用。C# などは需要が出てから）
- 配布の形（MSIX か、署名付きの exe と installer か）と、自動アップデート（macOS の Sparkle 2 に相当するもの）

## Status log

- 2026-10-19: nanoKONTROL2 で Windows MIDI Services の multi-client と SysEx の通り方を実測（`mem_1CgBB8Xpu4BLF1v1hDFtAY`）。`cargo check` を Windows で実行し、`midistage-protocol` / `midistage-client` / `midistage-profiles` は通った。止まるのは `midistaged` の `settings.rs`（`std::os::unix`・`libc::flock`）と、`midistage-core` の `#[link(kind = "framework")]` だけ。この design を起草
