# connect_tcp で IPv4 を IPv6 より優先している

Created: 2026-03-18
Completed: 2026-04-07
Model: GPT-5.4

## 概要

`connect_tcp` で IPv4 アドレスを IPv6 アドレスより優先してソートしている。
Happy Eyeballs (RFC 8305) に準拠していないため、IPv6 環境での接続遅延の原因になり得る。

## 該当箇所

- `src/client.rs:2240-2257`

## 優先度

低

## 解決方法

`connect_tcp` の IPv4 優先ソートを削除し、`lookup_host` が返す順序をそのまま
`TcpStream::connect` に渡すように修正した。
