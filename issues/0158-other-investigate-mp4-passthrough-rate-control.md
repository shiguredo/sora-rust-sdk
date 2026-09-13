# MP4 パススルーの rate-control 契約を調査する

- Created: 2026-09-11
- Completed: {YYYY-MM-DD}
- Branch: feature/debug-mp4-passthrough-rate-control
- Polished: {YYYY-MM-DD}

## 目的

MP4 パススルーが libwebrtc に申告する rate-control 能力と実際の動作を照合し、圧縮済み sample の参照関係を壊さずに送信帯域へ対応する方針を決める。

調査結果からコード変更が必要だと判断した場合は、単一の実装方針に絞った別 issue を起票する。

## 現状

`src/video_codecs/mp4.rs` の `Mp4PassthroughEncoder::get_encoder_info` は `has_trusted_rate_controller=true` を返す。

`Mp4PassthroughEncoder::set_rates` は target bitrate と framerate をログへ出力するだけで、送出量を変更しない。
MP4 パススルーは圧縮済み sample を再エンコードできないため、通常の encoder と同じ量子化や解像度変更による rate control は実装できない。

libwebrtc の `api/video_codecs/video_encoder.h` にある `VideoEncoder::EncoderInfo::has_trusted_rate_controller` は、encoder が target bitrate の近くで動作し、必要なら frame drop を行い、rate change へ速やかに反応することを前提とする。
現在の `Mp4PassthroughEncoder` は、この契約どおりの制御を行っていない。

一方、`has_trusted_rate_controller=false` に変更するだけでは、libwebrtc の frame dropper が圧縮済み sample を codec dependency と無関係に破棄し得る。
reference frame だけが破棄されると、後続の delta frame をデコードできない。

libwebrtc の `VideoStreamEncoder::OnFrame` と `VideoStreamEncoder::MaybeEncodeVideoFrame` には、`has_trusted_rate_controller` の値にかかわらず encode 前の frame を破棄する経路もある。
どの drop 経路を encoder 情報で制御できるかを分けて確認する必要がある。

Sora の issue 0190 で扱った E2E は、約 2 Mbps の VP9 MP4 を sumomo の `--video-bit-rate` 未指定で送信する。
Sora の既定 target bitrate は 500 kbps であり、入力 bitrate と target bitrate に約 4 倍の差がある。
この条件では pacer backlog と keyframe flushing が発生しているが、MP4 パススルーの rate-control 申告との因果関係は確定していない。

## 調査方針

### libwebrtc の契約と分岐

SDK が使用する libwebrtc について、次の呼び出しと条件をソースコードで確認する。

- `VideoStreamEncoder` が `VideoEncoder::SetRates` を呼ぶ条件
- `has_trusted_rate_controller` が media optimization の frame dropper に与える影響
- queue overload、congestion window pushback、送信停止など、encoder 情報では抑止できない encode 前 drop
- pacer queue の backlog と keyframe flushing が発生する条件

確認結果はファイルパスとシンボル名で issue に記録する。

### 実 MP4 による観測

実 MP4 fixture と sumomo を使用し、入力の平均 bitrate と peak bitrate に対して target bitrate を上、同程度、下に設定して比較する。

各条件で次を記録する。

- `SetRates` へ渡される target bitrate と framerate
- capturer が供給した sample index と encoder callback へ到達した sample index
- keyframe flag と keyframe request
- 送信 RTP の timestamp、marker、sequence number
- pacer delay、送信 bitrate、不完全フレームの発生数

恒久的なフレーム単位ログを追加することは本 issue の目的にしない。
調査用ログが必要な場合は `feature/debug-` ブランチだけで使用し、通常運用へ残すログは実装 issue で改めて判断する。

### 方針の決定

調査結果から、少なくとも次の候補を比較して 1 つに絞る。

1. MP4 パススルー側で dependency-safe な sample 抑止を行い、trusted rate controller として target bitrate に追従する。
2. `has_trusted_rate_controller=false` に変更し、sora-rust-sdk の issue 0157 による keyframe 復帰と組み合わせる。
3. パススルーでは target bitrate への追従を保証せず、入力 bitrate と送信設定の不整合を接続前に検出して拒否または警告する。

再エンコードを必要とする方針は MP4 パススルーの目的に反するため採用しない。

## 完了条件

- `has_trusted_rate_controller` が libwebrtc の各 frame drop 経路へ与える影響がシンボル単位で整理されていること
- `has_trusted_rate_controller=true` でも `SetRates` が呼ばれるかどうかを確認していること
- 入力 bitrate と target bitrate を変えた実 MP4 配信の測定結果が記録されていること
- pacer backlog、keyframe flushing、encode 前 drop を区別できていること
- 圧縮済み sample の参照関係を壊さない rate-control 方針が 1 つに決まっていること
- コード変更が必要な場合は、変更対象と完了条件を確定した実装 issue が起票されていること
- モックやスタブを使用しないこと

## 変更対象

調査対象は次のとおりである。

- `src/video_codecs/mp4.rs` の `Mp4PassthroughEncoder::set_rates`
- `src/video_codecs/mp4.rs` の `Mp4PassthroughEncoder::get_encoder_info`
- `examples/sumomo/src/args.rs` の video bitrate 設定
- libwebrtc の `VideoEncoder::EncoderInfo`
- libwebrtc の `VideoStreamEncoder`
- libwebrtc の `PacingController`

## 関連 issue

- sora-rust-sdk の 0157: sample 欠落と keyframe request の後で次の実在する keyframe へ復帰する。
- Sora の issue 0190: VP9 E2E の末尾デコード失敗と MP4 パススルー固有の経路を調査した。
