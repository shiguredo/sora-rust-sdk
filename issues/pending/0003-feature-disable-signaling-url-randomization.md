# シグナリング URL ランダム化の無効化

## 概要

シグナリング URL のランダム化を無効化できるようにする。

## 背景

C++ SDK には `disable_signaling_url_randomization` オプションがあり、URL リストのシャッフルを無効化できる。
Rust SDK ではデフォルトでランダム化が有効だが、無効化する手段がない。

## 現状

`SoraClient::run()` 内で `urls.shuffle(&mut rand::rng())` を呼んでいる。
ランダムソースとして `rand` クレートを使用しているが、`aws-lc-rs` の `SystemRandom` も利用可能。

## 対応内容

- `SoraClientBuilder` に `disable_signaling_url_randomization: bool` を追加する
- `SoraClient::run()` で `disable_signaling_url_randomization` が `false` の場合のみシャッフルする
- `docs/SORA_CPP_SDK.md` の対応表を更新する

## pending 理由

ファーストリリースでは実装しない。優先度が低い。
