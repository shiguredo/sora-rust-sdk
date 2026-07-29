# V4L2 encode callback の再登録を同期する

- Priority: High
- Created: 2026-07-29
- Completed: {YYYY-MM-DD}
- Model: GPT-5
- Branch: feature/fix-v4l2-encode-callback-race
- Polished: 2026-07-29

## 目的

V4L2 の非同期 encode callback と callback 再登録を同期し、寿命が終了した callback pointer の呼び出しを防ぐ。

## 優先度根拠

High。callback の寿命契約に反する raw pointer 呼び出しが成立し、並行再登録時に不正メモリアクセスへつながり得る。

## 現状

`handle_v4l2_encode_callback` は shared state の lock 内で callback pointer をコピーし、lock を解放してから呼び出す。
`V4L2Encoder::register_encode_complete_callback` は、進行中 callback の完了を待たずに pointer を置き換える。

`VideoEncoderEncodedImageCallbackPtr` は所有権を持たない `Copy` pointer である。
依存する `shiguredo_webrtc` の safety contract は、pointer の参照先を呼出中も有効に保ち、register の再呼出しまたは release 後に使用しないことを要求する。

本リポジトリが固定する `shiguredo_webrtc` の libwebrtc は `m150.7871.3.1` である。
対応する libwebrtc branch-head の commit `1f975dfd761af6e5d76d28333191973b258d82a8` では、`video/video_stream_encoder.cc` に次の所有・呼出順序がある。

- encoder の初期化成功後、encoder queue 上で `encoder_->RegisterEncodeCompleteCallback(this)` を呼び、callback owner である `VideoStreamEncoder` 自身を登録する
- `ReleaseEncoder` は同じ encoder queue 上で `encoder_->Release()` を同期的に呼ぶ
- `Stop` は encoder queue に `ReleaseEncoder` を含む teardown task を投入し、その完了を待ってから `VideoStreamEncoder` の破棄へ進む
- `VideoStreamEncoder::OnEncodedImage` は register / release を同期的に呼ばず、encoder queue への後続 task 投入と sink への配送を行う

したがって固定中の production call path では、register / release の呼出中と V4L2 poller callback の完了待ち中も callback owner は生存し、callback handler から register / release へ同期再入しない。
この upstream 前提は raw pointer の safety proof に含め、libwebrtc 更新時には該当 call path を再検証する。

現在は次の競合が成立する。

1. V4L2 poller thread が lock 内で旧 callback pointer をコピーする
2. 別 thread が新 callback を登録し、旧 callback の寿命が終了する
3. poller thread が lock 外で旧 pointer を呼び出す

## 設計方針

### 対象範囲

- `src/video_codecs/v4l2.rs` の encode-complete callback pointer だけを対象とする
- V4L2 convert callback は raw WebRTC callback pointer を保持しないため対象外とする
- V4L2 decoder と AMF / VPL / NVCodec などの他 backend を共通 abstraction へ移行しない
- 他 backend の callback pointer は必要に応じて別 issue で監査する

### callback 同期 state

encoder と converter の所有 state から、encode-complete callback 専用の同期 state を分離する。
固定する `shiguredo_v4l2` 2026.1.0 の `src/encoder.rs` では handler を一つの poller thread closure が単独所有するため、V4L2 encoder の completion は直列に通知される。
`shiguredo_webrtc` の callback object は `Send` だが `Sync` ではないため、同一 callback object の外部呼出しは直列のまま維持する。
同期 state は `Mutex`、`Condvar`、active generation、その世代の in-flight 有無を保持する。

- callback pointer の取得と、その世代を in-flight にする操作を同じ lock の critical section で行う
- active generation が更新中または未登録なら frame を破棄し、pointer を取得しない
- pointer を取得した後は lock を解放してから外部 callback を呼ぶ
- callback 呼出しごとに RAII guard を作り、正常 return と callback の非 `Ok` result で該当世代の in-flight を解除して waiter を notify する
- `shiguredo_webrtc` の callback handler trampoline は non-unwind の `extern "C"` 境界であるため、handler panic 時は既存どおり process abort となる
  - panic からの復帰と in-flight 復旧は保証せず、`catch_unwind` も追加しない
- in-flight に generation を記録し、新旧 generation を混在させない
- `Some` / `None` の register と release は同じ update ownership で直列化し、`Condvar` は predicate loop で待機して spurious wakeup を許容する
- generation 番号を再利用する場合は、旧世代の entry と in-flight が完全に消滅した後だけにする
- 同一 callback object への複数 thread からの並行呼出しを新たに許可しない

同期 register の return 前に旧世代の quiescence を保証する必要があるため、callback state を mpsc の単一 owner へ送るだけの設計にはしない。
外部 callback、encoder / converter の Drop、poller join は callback-state lock と encoder-state lock の外で行う。

### register の線形化と保証

`register_encode_complete_callback(Some(new))` と `register_encode_complete_callback(None)` は同じ barrier として処理する。
同一 pointer の再登録も寿命境界として省略しない。

1. callback state の lock 内で active generation を retiring にし、旧世代の新規取得を停止する
2. lock を解放する `Condvar` wait で旧世代が in-flight でなくなるまで待つ
3. 旧世代の entry を削除する
4. `Some(new)` なら新 generation と pointer を publish し、`None` なら未登録のままにする
5. register 操作の完了を publish して return する

- register 開始前に取得済みの旧 callback は、register が返る前に完了する
- retiring 開始後に完了した frame は、旧 callback にも未 publish の新 callback にも渡さず破棄する
- register が返った後は、旧 pointer を新規取得または dereference しない
- 新 callback は publish 後に取得した frame だけを受け取る
- encode request の enqueue 時ではなく encode 完了時の active callback を選ぶ現行 semantics を維持する

### callback からの再入

libwebrtc m150 の `VideoEncoder` interface は register / release の同期的 callback 再入を要求しておらず、固定 commit の production call path にも再入はない。
`shiguredo_webrtc` の非所有 pointer にも再入中の寿命を延長する所有機構はない。
このため、本 issue の安全性保証は control thread からの register / release と poller thread の callback 呼出しの競合を対象とし、callback handler 自身からの同期的な register / release は契約外とする。

- 外部 callback を callback-state lock と update ownership の外で呼び、register / release 以外の処理には新たな lock 再入を発生させない
- callback thread を記録し、同じ callback state への同期的な register / release 再入は、更新や待機を始める前に `VideoCodecStatus::Error` で防御的に拒否する
- この拒否を pointer 寿命保証の根拠にはしない
  - 呼出側は進行中 callback が return するまで登録済み callback の寿命を維持する、という既存の libwebrtc 呼出順序を前提とする
- libwebrtc または `shiguredo_webrtc` が同期的再入を正式に要求する場合は、callback の所有権を含む別 issue で対応する
- callback thread からの release では encoder / converter を変更せず、poller thread が自分自身を join しないようにする

### release の teardown

通常 thread からの `release` は、register と共有する update ownership を取得してから次の順序にする。

1. register の `None` と同じ barrier で active callback を retiring にし、新規取得を止める
2. 先行 in-flight がなくなるまで待ち、callback pointer を削除する
3. `shared_state.encoder` を `take` する
4. libcamera converter と encoder を既存の依存順に従い、callback-state lock と encoder-state lock の外で Drop する
5. poller の drain / join が完了してから `release` を返す

`release` の線形化点より前に登録された世代について、返却後は callback の新規開始と実行中 callback がともになく、その pointer を一切使用しない。
encoder Drop が drain callback を発生させても active callback は既に `None` のため、外部 callback は呼ばない。
既存の rebuild 経路が encoder を lock 外で Drop し、旧 converter callback と新 encoder の混在を防ぐ順序も維持する。

register と release の競合は、update ownership を先に取得した操作を先行操作として線形化する。

- register が先行した場合、callback publish の完了後に release がその世代を retire し、release 返却時は未登録になる
- release が先行した場合、競合した register は release 完了後に新世代として callback を publish する
- register を encoder 初期化状態では拒否せず、現行の register と `init_encode` の順序を維持する
- したがって release の postcondition は release より前の登録世代だけを対象とし、release 後に線形化された明示的な register は新しい寿命境界として扱う

## 変更対象

- `src/video_codecs/v4l2.rs`
- `CHANGES.md`

## 完了条件

- private callback 同期 state の unit test で、実際の `VideoEncoderEncodedImageCallback`、`VideoEncoderEncodedImageCallbackPtr`、`EncodedImage`、`CodecSpecificInfo` を使用する
- thread、`Barrier`、channel、timeout を使う決定的なテストで次を確認する
  - 旧 callback handler を停止している間、別 thread の `Some(old) -> Some(new)` が返らない
  - register が返った後に旧 callback object を Drop しても、次の frame は新 callback だけへ届く
  - `Some -> None` の返却後は callback が呼ばれない
  - `None -> Some` の publish 後は新 callback が呼ばれる
  - 同一 pointer の再登録も先行 in-flight を待つ
  - 連続する register 操作が世代を混在させず完了する
  - callback の非 `Ok` result path でも RAII guard が in-flight を解除する
- V4L2 completion の単一 poller thread という前提を production code のコメントで明記し、同一実 callback object を複数 thread から同時に呼ぶテストを追加しない
- 実 callback handler 内から同期 state の実 register 経路へ `Some` と `None` を再入させ、いずれも timeout 内に `VideoCodecStatus::Error` を返して state を変更しないことを確認する
- 実 callback handler 内から release の事前判定経路へ再入させ、timeout 内に `VideoCodecStatus::Error` を返し、encoder / converter / callback state を変更しないことを確認する
- 通常 thread の release について次を確認する
  - 先行 callback が完了するまで返らない
  - return 後は in-flight と callback 呼出回数が増えない
  - encoder が未初期化でも冪等に成功する
- register と release の決定的な競合テストで、次の両順序を barrier で固定して確認する
  - register が先行した場合は release が新世代も retire し、release 返却後は未登録になる
  - release が先行した場合は register が release 完了後に新世代を publish し、その callback を呼び出せる
- 競合テストは sleep の順序へ依存させず、mock / stub、`#[ignore]`、V4L2 device 不在時の skip を使用しない
  - private 同期 state と実 WebRTC callback object を使うため、`/dev/video*` は不要とする
- self-hosted Raspberry Pi CI の次の既存 command で新テストが実行される
  - `cargo clippy --workspace --features openh264,libcamera,v4l2 -- -D warnings`
  - `cargo test --workspace --features openh264,libcamera,v4l2`
- default feature の `cargo test --workspace` も成功する
- callback pointer を dereference する `unsafe` block の safety comment に、lease と register / release barrier が寿命を保証する理由を日本語で記載する
- safety comment に libwebrtc commit `1f975dfd761af6e5d76d28333191973b258d82a8` の `video/video_stream_encoder.cc` における callback owner の寿命と非再入の前提、および libwebrtc 更新時に再検証が必要なことを記載する
- callback state の lock、外部 callback、encoder Drop、`Condvar` wait の順序を日本語コメントで記載する
- `CHANGES.md` の develop セクションに `[FIX]` を追記する
- production log は英語、コメントとテストの assertion message は日本語にする
