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

`native_midi` 拡張には、lease ごとにアプリ専用の仮想 source / destination が要る（design 03）。WinMM には仮想ポートを作る API が無いので、仮想 endpoint を作れる Windows MIDI Services の API（WinRT、`Windows.Devices.Midi2`）を使う。UMP がそのまま通るので、`midistage-core` の UMP 型とも合う。`windows` crate に入っている `Windows.Devices.Midi` は旧来の MIDI 1.0 用 API で、仮想 endpoint を作れないので使わない。

### `Windows.Devices.Midi2.dll` を同梱する（mako 裁定 2026-10-19）

API が Windows に入るまでは、NuGet パッケージの dll と pri を midistaged.exe の隣に置く。Microsoft の go-live 条件（下の spike）を受け入れる。25H2 未満の一般向け Windows 11 はサポートが終わる時期なので、対象の制約は実質ない。

- 取得: `scripts/winmidi.sh fetch`（版と sha256 を script に固定。`.cache/winmidi/` に winmd / dll / pri）。`crates/midistaged/build.rs` が dll と pri を `target/<profile>/` に写す
- binding: `scripts/winmidi.sh bindings` が `tools/winmidi-bindgen`（workspace の外）で `crates/midistaged/src/winmidi_bindings.rs` を作る。使う型だけに絞り（今は 4 型、1183 行）、生成物は commit する。bindgen の `[NoException]` static の不具合は、生成後に戻り値を `windows_core::Result<T>` に直して回避する（dll が無いときに panic せず理由を返すため）
- dll が無ければ `list-ports` は理由を言って失敗する

### ポートの ID

`inventory::Port` は CoreMIDI の整数 ID（`uid` / `physical_device`）を持つ。Windows の endpoint は双方向で、ID は文字列（`\\?\swd#midisrv#…`）。endpoint 1 つを IN / OUT の 2 ポートにし、`uid` は小文字にした endpoint ID + `#in` / `#out` の FNV-1a、`physical_device` は親デバイス ID（`USB\VID_…&PID_…\<serial または挿し口>`）の FNV-1a とする。transport が `KS` / `KSA`（実機のドライバ）でない endpoint（`APP` / `LOOP` / `DIAG` など）は物理ポートとして扱わない。シリアルの無い機材は親 ID に挿し口が入るが、保存の鍵は profile の device ID なので、挿し替えても割り当ては保たれる。送信・受信で `uid` から endpoint ID を引く表は 3 段目で持つ。

### OS に依らない段取りは `native_runtime`、OS の呼び出しは backend

`native.rs`（macOS）は、CoreMIDI の呼び出しと、OS に依らない段取り（機材の出入り、lease ごとの bridge の開閉、引き継ぎの完了待ち、送信の直列化と lease の世代での認可、finalizer）を 1 ファイルに持っている。Windows はこの段取りを `native_runtime.rs`（cfg 無し）に移し、OS の呼び出しを 2 つの trait の向こうに置いた。

- `Backend`: `discover`（read-only の列挙）と `open`（lease の持ち主専用の bridge を作る。実機を開くのはここだけ）
- `Bridge`: `fence` / `ports` / `output_sinks` / `finish_release`。drop で閉じる
- `output_worker(…, send)`: 送信の直列化。`send` は物理 destination へ 1 メッセージ送り、ドライバが受け取るまで戻らない

偽の backend で、bridge を開く条件、抜線後は Quiesced まで閉じないこと、finish の失敗で releasing に留まること、列挙の失敗で落ちないことをテストする。**macOS の `native.rs` はこの PR では変えない**（Windows から macOS 向けの型チェックができない。ring の C コードに clang と Mac の SDK が要る）。macOS を `native_runtime` に載せ替えるのは、macOS で検証できる別 PR。

### 送信の完了は `WaitForEndpointReceiptOnSend`

物理 endpoint には `MidiEndpointConnectionSettings::CreateInstance(waitForEndpointReceiptOnSend = true)` でつなぐ。送信 API はドライバが受け取るまで戻らない（0x7D の SysEx 67 packet ≈ 400 byte が、false で 0.8 ms、true で 19.5 ms。実測 2026-10-19）。これを CoreMIDI の SysEx completion 相当として扱う。後始末の送信も同じ接続を使うので、`MIDIFlushOutput` に当たる手順は要らない。タイムアウトだけで先へ進めない、という不変条件は変えない。

### bridge（仮想デバイス）

- 物理 endpoint 1 つにつき仮想デバイス 1 つ（双方向、MIDI 1.0 protocol、group 1 つ、function block 1 つ）。アプリは client 側を開き、サービスが device 側（`AddMessageProcessingPlugin`）と物理 endpoint の間を中継する
- 名前は `Midistage/<6 桁の札> <機材名>`（`inventory::windows_bridge_name`）。札は client / lease token / uid の FNV-1a で、lease ごとに変わる。WinMM の名前は 32 WCHAR（NUL 込み）で切れるので、UTF-16 で 31 単位に収め、新 API のアプリにも WinMM のアプリにも同じ名前で見せる
- 実機 → アプリ: UMP のまま、元の timestamp で中継する（lease が有効な間だけ）。アプリ → 実機: `midistage_core::midi1::Decoder` で MIDI 1.0 メッセージに組み立て直して送信 worker に積み、`midi1::encode` で UMP に戻して送る。UMP の utility（MT 0）と stream（MT F。service が仮想デバイスに送ってくる endpoint discovery など）は中継しない
- group 0 だけを扱う。複数 group の機材（Keystage の CTRL ポートなど）は未対応
- 物理 endpoint の列挙（`FindAll`）は 1 回 約 60 ms かかるので、1 秒ごとにして間は前回の結果を使う

### 今の midisrv は仮想デバイスを閉じると固まる → bridge は opt-in

Windows 11 26200 の in-box transport（`Midi2.VirtualMidiTransport.dll` 1.0.15.0）は、仮想デバイスの接続を閉じると `midisrv` が応答しなくなる（microsoft/MIDI#1047。実測 2026-10-19 に 3 回）。以後は誰が `CreateVirtualDevice` を呼んでも戻らず、`midisrv` の再起動（強制終了か PC の再起動。止める合図には応えない）まで、仮想ポートを使うすべてのアプリが止まる。修正は Windows の更新（11 月末の 11D、有効になるまで最大 45 日）で届き、同梱の API（1.0.16）では直らない。仮想デバイスを閉じずにプロセスが終わった場合は、その後も作れた（2 回）。

- `serve --experimental-windows-bridge` のときだけ bridge を作る。既定では作らず、機材の error に理由と試し方を出す（空の成功にしない）。lease と状態の管理は既定でも動く
- `open` は別 thread で時限（10 秒）付き（`watchdog::Watchdog`）。戻らなければエラーにし、止まった呼び出しが残る間は次を始めない（止まった thread を積み重ねない）。引き継ぎの完了待ちには時限を使わない
- 接続を外す呼び出しは別 thread に逃がす。中継と送信は先に止めているので、戻らなくても native の worker は止まらない
- 「閉じた仮想デバイスが一覧から消えるまで待ってから次を開く」待ち時間も試したが、閉じる側で固まるので効かなかった（外した）

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
3. **native 本体**: 物理 In / Out、仮想 endpoint、送信完了の意味の確認、引き継ぎ。nanoKONTROL2 から（design 03「観測と移行」と同じ順）。**bridge は Windows の修正待ちで opt-in**（上記）
4. **常駐と配布**: タスクトレイ app、署名、登録
5. **ROTO の Serial**（design 06 の Windows 版）

1 は今のコードの延長でできる。2 から先は、Windows MIDI Services の SDK を Rust から呼べることを先に確かめる（spike）。

## 未決

- bridge の実機確認（Windows の 11D 修正の後）: A → B の引き継ぎを通しで、VP で nanoKONTROL2 から lane を選ぶ、旧アプリの遅延送信が実機に流れないこと。確認できたら opt-in を外す
- macOS の `native.rs` を `native_runtime` に載せ替える（macOS で検証できる別 PR）
- 複数 group の機材（Keystage の CTRL ポートなど）の bridge
- `GetAssociatedClientEndpointDeviceId(config.AssociationId())` が空を返した（spike）。今は名前でポートを選ぶので使っていない
- KORG USB-MIDI Driver を入れた機材が、`midisrv` からどう見えるか（Zenith 2 は `MIDIU_KSA_…` の経路だった）
- Windows のアプリ向けの SDK（Rust SDK は使える。Swift SDK は macOS 専用。C# などは需要が出てから）
- 同梱をやめる時期の判定（`MidiApi::IsSystemProvided()` が true の Windows が十分に行き渡ったら）
- 配布の形（MSIX か、署名付きの exe と installer か）と、自動アップデート（macOS の Sparkle 2 に相当するもの）

## spike: Rust から Windows.Devices.Midi2（2026-10-19）

- API は `Windows.Devices.Midi2`。2026-10-12 の In-box Preview 11 で metadata が凍結され、将来は Windows に入る。このマシン（26200）にはまだ無い（`MidiApi::IsSystemProvided() = false`）
- NuGet パッケージ `Windows.Devices.Midi2 0.99.92-preview.11`（MIT、GitHub Releases のみ。nuget.org には無い）の winmd（415 KB）から `windows-bindgen 0.62` で binding を作り、`Windows.Devices.Midi2.dll`（6.9 MB）と `.pri` を exe の隣に置けば動く。SDK Runtime / Tools のインストーラー（管理者）は要らない。OS に入った後は OS 側が優先される
- windows-bindgen（0.62・0.100 とも）は `[NoException]` の static メソッドを、`Result` を期待する factory の closure の中で素の値を返す形に生成し、コンパイルが通らない（349 箇所）。生成後に `Ok(..)` で包む置換で回避した。0.100 の生成物は `windows-core 0.100` とも合わない
- 列挙の結果（何も開かず、何も送らない）: endpoint は双方向で 1 機材 1 つ。`GetParentDeviceInformation()` から USB の VID / PID / シリアルが取れる（nanoKONTROL2 は `0944:0117`、シリアル無し。Zenith 2 は `23E5:A2F4`、シリアル有り）。`SupportsMultiClient = true`、transport は nanoKONTROL2 が `KS`（usbmidi2）、Zenith 2 が `KSA`（ベンダードライバ）
- go-live 条件（Preview 11 のリリースノート）: Windows 11 25H2 以上にだけ配る、API が Windows に入ったら同梱を外す更新を出す、preview API を使っていることを利用者に伝える、新しい preview に追従する、など

## Status log

- 2026-10-19: nanoKONTROL2 で Windows MIDI Services の multi-client と SysEx の通り方を実測（`mem_1CgBB8Xpu4BLF1v1hDFtAY`）。`cargo check` を Windows で実行し、`midistage-protocol` / `midistage-client` / `midistage-profiles` は通った。止まるのは `midistaged` の `settings.rs`（`std::os::unix`・`libc::flock`）と、`midistage-core` の `#[link(kind = "framework")]` だけ。この design を起草
- 2026-10-19: 1 段目。`serve` が Windows で起動して留まる（MIDI backend 無し、機材は waiting）。排他は std の `File::try_lock`（unix は従来どおり flock）、置き場は `%LOCALAPPDATA%\Midistage`、終了の合図は Ctrl-C / Break / close / logoff / shutdown。`list-ports` / `roto-dump` は成功扱いにせず失敗で返す。`cargo test --workspace` と clippy が Windows で通る（Rust SDK の相互テスト 7 件を含む）。Windows での Ctrl-C からの正常終了（`endpoint.json` の削除）は手元で未確認。`.gitattributes` で作業ツリーを LF に固定した（`core.autocrlf` でログ契約の fixture が CRLF になり、バイト比較のテストが落ちたため）
- 2026-10-19: 2 段目。`list-ports` が Windows MIDI Services で物理ポートを列挙する（read-only）。手元で nanoKONTROL2 → `nanokontrol`、Zenith 2 → `zenith2` の IN / OUT を確認。dll が無い exe では理由を言って失敗することを cli テストで確認。`cargo test --workspace` / clippy / fmt は Windows で通る。macOS 側のビルドとテストは未確認
- 2026-10-19: 3 段目。`native_runtime`（OS に依らない段取り、偽 backend のテスト付き）、`midistage_core::midi1`（MIDI 1.0 ⇄ UMP）、`winmidi` の bridge（仮想デバイス + `WaitForEndpointReceiptOnSend`）、`watchdog`。引き継ぎを手で試す `midistage-client` の example `try_lease`（アプリ B の代役。Ladyland に Windows 版が無いため）。実機（nanoKONTROL2）: A の bridge open（約 135 ms）→ SendMidi の Device Inquiry 応答が bridge 入力に届く → B の takeover で A に Quiesce → A が Quiesced → A の bridge close、まで通った。close で midisrv が固まり（microsoft/MIDI#1047）、B 以降は実機確認待ち。bridge を opt-in にし、既定では機材の error に理由を出すことを実機で確認（警告は 1 回だけ）
