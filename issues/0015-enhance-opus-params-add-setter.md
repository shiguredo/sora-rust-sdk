# OpusParams のフィールドに setter を追加する

## 概要

`OpusParams` のフィールドが全て private で setter がない。
`Default` は実装されているが、全フィールドが `None` の状態しか作れず、利用者が `OpusParams` を構成する手段がない。

## 該当箇所

- `src/types.rs:96-105`

## 優先度

低
