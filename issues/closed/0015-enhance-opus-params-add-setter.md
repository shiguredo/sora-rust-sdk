# OpusParams のフィールドに setter を追加する

Created: 2026-03-18
Completed: 2026-04-07
Model: GPT-5.4

## 概要

`OpusParams` のフィールドが全て private で setter がない。
`Default` は実装されているが、全フィールドが `None` の状態しか作れず、利用者が `OpusParams` を構成する手段がない。

## 該当箇所

- `src/types.rs:96-105`

## 優先度

低

## 解決方法

`OpusParams` と `VideoVP9Params` / `VideoH264Params` / `VideoH265Params` /
`VideoAV1Params` のフィールドを `pub` に変更し、利用者が構築できるようにした。
あわせて `Audio::new_opus` と `Video::new_vp9` / `new_av1` / `new_h264` /
`new_h265` でパラメータが JSON に反映される単体テストを追加した。
