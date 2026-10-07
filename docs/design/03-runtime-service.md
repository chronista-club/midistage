# 03. アプリと MIDI 機材の共通サービス

> **Status**: Implementing（共通サービス基盤・SDK。アプリ移行は後続）
> **Related**: `mem_1CfnVkncPrmfdQet3bNSmx`、design 01
> **対象**: `schemas/midistage.kdl`, `crates/midistage-protocol`, `crates/midistaged`, `crates/midistage-profiles`, `clients/swift`

midistaged が実機 MIDI 接続を持ち、Ladyland / VP は対等な client になる。機材の発見・能力・入力・表示・使用権・引き継ぎを共通契約にする。Track / lane の意味はアプリ側に残す。各アプリが直接実機を開く方式は採用しない。

## 分割

- protocol: I/O を持たない型・状態遷移。KDL と wire 型・共通 fixture を同時に更新する。
- midistaged: CoreMIDI、アプリセッション、保存、Unison、送信の直列化と完了待ち。
- profiles: 機材固有の入力・表示変換。VP 内に仮置きされていた crate をここに集約する。
- SDK: Rust / Swift から接続・購読・使用設定・引き継ぎ・表示更新を行う窓口。

## 制御と MIDI データ

制御は Unison の `midistage` channel。loopback の動的 port にだけ bind し、user-private な endpoint.json で証明書 pin と認証 token を配る。runtime.lock の flock で単一起動を保証。アプリはサービスを自動的にバージョン入れ替えしない。

新規アプリ向けの入力は操作子 ID + absolute / relative / button / touch を使う。表示は `Present` で name / value / color を渡す。対応能力は snapshot の controls に列挙し、表現できない指定は失敗として返す。

この段階で実装済みなのは使用権・状態購読・native MIDI 拡張。操作イベントと `Present` の型は共通契約に含むが、runtime の semantic adapter は後続。未実装の表示要求を成功扱いしない。profiles の純粋変換テストが通ることと、サービスから実機への表示確認を区別する。

既存 Ladyland / VP の移行には `native_midi` 拡張を用意する。サービスが使用権に紐づいたアプリ専用の仮想 MIDI source / destination を作り、OS の MIDI 経路で既存演奏・SysEx 処理を維持する。物理ポートはサービスだけが開く。アプリは自分の bridge ポート以外を選ばない。bridge の送信口も lease の世代で閉じ、旧アプリの遅延送信を実機へ流さない。正常化された操作イベントと native MIDI の両方を同じ意味付けへ重複投入しない。

LED frame など完了待ちで次を送る用途は `SendMidi` を使い、物理 CoreMIDI の SysEx completion まで応答を待つ。仮想 destination の SysEx completion はサービスへの受け渡し完了にすぎず、実機の送信完了として扱わない。要求には lease token と snapshot に列挙されたポート名が必要で、単一の完全な MIDI message のみ受け付ける。

## 状態

presence（実機検出）、assignment（保存された担当と revision / expected）、lease（実行セッションと起動世代付き token）を分離する。phase は off / waiting / active / releasing / error。

`active(A) → releasing(A, B) → active(B)` の間、新規入力と出力を停止する。A に Quiesce を通知し、A はその入力由来の発音を整理して Quiesced を返す。サービスは待機中の送信を破棄し、送信中の SysEx 完了を確認してから A の bridge を閉じ、B の bridge と新 lease を作る。A の停止確認前に B を active にしない。タイムアウトだけで active に進めない。

物理切断も cleanup を通して waiting へ遷移する。セッション切断では lease を破棄するが assignment を保持し、再接続で復元する。サービス再起動で token は必ず変わる。OFF は保存され、挿し直しや初回既定設定で上書きしない。初回移行の既定 ON は既存 owner を奪わない。

v1 は一機種につき一台。CoreMIDI の物理 device ID で Keystage の複数ポートをまとめ、保存先は profile に対応する device ID とする。Hub 交換で endpoint ID や順序が変わっても割り当てを保持する。同一機種が複数台なら接続を止めて error を表示する。未知の機種や他アプリの仮想ポートを既知機材と誤認しない。

仮想ポートが開くまで snapshot は waiting。releasing 中は旧 lease を停止通知の宛先としてだけ残す。Unison の event は混雑時に欠落しうるため、状態は定期再送、Quiesce は停止確認まで再送する。演奏データは native MIDI で元の入力 timestamp を保持する。出力は即時制御用で、アプリの将来時刻の MIDI 演奏スケジューラとしては扱わない。

設定は一時ファイルへの書き込み・fsync・rename 後に状態へ反映する。保存失敗で現 owner を失わない。物理出力は lease で認可し、SysEx completion と短いメッセージの flush が完了するまで後任へ渡さない。driver が完了を返さない場合は releasing のままにする。

## 観測と移行

最初に nanoKONTROL の 8 fader / 8 knob / S・M・R / bank を通す。次に LPD8、ROTO。既存機材のシーン設定や保存内容を自動で書き換えない。ROTO の DAW / MIDI モード・アプリ固有のページ構成は、共通表示 API へ移すまで既存 adapter を native MIDI 拡張で保持する。

自動テストは模擬 backend と実 Unison 接続で、認証・所有権・stale token・再接続・保存・送信停止の順序を検証する。実機の音・遅延・LED / motor は別途観測する。

## Status log

- 2026-10-07: 共通サービス方式を承認。直接接続 + アプリ間 flock 案を廃止し、protocol / daemon / profile / SDK の分割で着手。
- 2026-10-07: Rust / Swift の実 Unison 接続で認証、確認付き引き継ぎ、旧 lease 拒否、明示 cleanup ack を検証。CoreMIDI bridge はコンパイル確認まで。ユーザーの Hub 交換後に物理ポートを read-only 列挙し、Keystage / LPD8 / nanoKONTROL2 / ROTO の対応を確認。実機 I/O・Ladyland / VP 統合は未確認。
