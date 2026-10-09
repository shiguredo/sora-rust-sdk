# MP4 パススルーのキーフレーム要求への対応を検討する

- Created: 2026-09-11
- Completed: {YYYY-MM-DD}
- Branch: feature/debug-mp4-passthrough-keyframe-request
- Polished: 2026-09-14

## 目的

MP4 パススルーで libwebrtc からキーフレームを要求された場合に、入力 MP4 の次のキーフレームを待つだけでよいか、再生位置を進めて応答を早める必要があるかを判断する。

再生位置を進める最適化は、必要性と復帰時間を確認してから別の実装 issue として扱う。

## 現状

`src/video_codecs/mp4.rs` の `Mp4PassthroughEncoder::encode` は `frame_types` を参照せず、入力 sample の `is_keyframe` だけで出力 frame type を決める。
このため、PLI などによるキーフレーム要求を受けても現在位置が delta sample ならそのまま送信し、入力 MP4 の次のキーフレームが自然に到着した時点で復帰する。

MP4 パススルーは圧縮済み sample から新しいキーフレームを生成できない。
入力 MP4 のキーフレーム間隔を短くすれば自然復帰までの時間を抑えられるが、許容できる復帰時間は確認できていない。

キャプチャラーの再生位置を次の sync sample へ進める場合は、encoder と capturer の協調、再生時計の更新、古い要求との競合を扱う必要がある。
さらに、直前のフレームから近すぎる時刻でキーフレームを供給すると、`AdaptedVideoTrackSource::adapt_frame` のフレームレート制御によって、そのキーフレーム自体が破棄される可能性がある。

encode 前の sample 欠落後に delta sample を抑止する安全策は別 issue に分離する。

## 調査方針

1. MP4 ファイル配信では、復帰時間を短くするために入力 MP4 のキーフレーム間隔を短めにする必要があることを制約として整理する。
2. キーフレーム要求から入力 MP4 の次のキーフレームが送信されるまでの時間を測定し、自然復帰で許容できるかを確認する。
3. 自然復帰で十分な場合は、キーフレーム要求を積極的に処理せず、制約の文書化だけにする。
4. 自然復帰では遅すぎる場合は、要求を次の実キーフレームまで保持する方法と、capturer の再生位置を進める方法を分けて検討する。
5. 再生位置を進める必要があると判断した場合だけ、即時復帰の実装 issue を別に起票する。

rate-control の宣言、target bitrate との不整合、pacer backlog は本 issue では扱わない。

## 完了条件

- 入力 MP4 のキーフレーム間隔と、キーフレーム要求後の自然復帰時間の関係が記録されていること
- 自然復帰と即時復帰のどちらが必要かが決まっていること
- 自然復帰で十分な場合は、短いキーフレーム間隔を推奨する制約が `docs/INPUT_MP4.md` に記載されていること
- 即時復帰が必要な場合は、最適化だけに限定した実装 issue が起票されていること
- encode 前の sample 欠落対策と rate-control の調査を本 issue に含めないこと
- モックやスタブを使用しないこと

## 変更対象

- `src/video_codecs/mp4.rs` の `Mp4PassthroughEncoder::encode`
- `src/video_codecs/mp4.rs` の `Mp4VideoCapturer::new`
- `docs/INPUT_MP4.md`

## 関連 issue

- sora-rust-sdk の 0158: MP4 パススルーの rate-control 契約を調査する。
- sora-rust-sdk の 0159: encode 前の sample 欠落後に delta sample を抑止する。
- Sora の issue 0190: VP9 E2E のデコード失敗と MP4 パススルー固有の sample 欠落経路を調査した。
