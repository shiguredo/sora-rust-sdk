# connect_tcp で IPv4 を IPv6 より優先している

## 概要

`connect_tcp` で IPv4 アドレスを IPv6 アドレスより優先してソートしている。
Happy Eyeballs (RFC 8305) に準拠していないため、IPv6 環境での接続遅延の原因になり得る。

## 該当箇所

- `src/client.rs:2223-2225`

## 優先度

低
