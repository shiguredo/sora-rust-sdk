//! JSON-RPC 2.0 over DataChannel の型と組み立て。
use std::time::Duration;

use nojson::{DisplayJson, RawJsonOwned};

use crate::error::Result;
use crate::types::JsonString;

/// RPC リクエストのオプション。
#[derive(Debug, Clone)]
pub struct RpcRequestOptions {
    /// `true` の場合はレスポンスを待たない (JSON-RPC 2.0 Notification)。
    /// デフォルト: `false`
    pub notification: bool,

    /// レスポンスの待機タイムアウト。
    /// `notification` が `true` の場合は無視される。
    /// デフォルト: 5 秒
    pub timeout: Duration,
}

impl Default for RpcRequestOptions {
    fn default() -> Self {
        Self {
            notification: false,
            timeout: Duration::from_secs(5),
        }
    }
}

/// JSON-RPC 2.0 のレスポンス。
#[derive(Debug, Clone)]
pub enum RpcResponse {
    /// 成功レスポンス。
    Success {
        /// result フィールドの JSON 値。
        result: JsonString,
    },
    /// エラーレスポンス。
    Error {
        /// エラーコード。
        code: i32,
        /// エラーメッセージ。
        message: String,
        /// 追加データの JSON 値。
        data: Option<JsonString>,
    },
}

impl RpcResponse {
    pub(crate) fn parse(text: &str) -> Result<(Option<u64>, Self)> {
        let raw = RawJsonOwned::parse(text)?;
        let value = raw.value();

        let id: Option<u64> = value
            .to_member("id")?
            .optional()
            .map(|v| v.try_into())
            .transpose()?;

        // error フィールドがあればエラーレスポンス
        if let Some(error_value) = value.to_member("error")?.optional() {
            let code: i32 = error_value.to_member("code")?.required()?.try_into()?;
            let message: String = error_value.to_member("message")?.required()?.try_into()?;
            let data: Option<JsonString> = error_value
                .to_member("data")?
                .optional()
                .map(|v| RawJsonOwned::json(|f| DisplayJson::fmt(&v, f)))
                .map(JsonString::from);

            Ok((
                id,
                RpcResponse::Error {
                    code,
                    message,
                    data,
                },
            ))
        } else {
            let result_raw = value
                .to_member("result")?
                .optional()
                .map(|v| RawJsonOwned::json(|f| DisplayJson::fmt(&v, f)))
                .unwrap_or_else(|| RawJsonOwned::json(|f| f.value("null")));
            let result = JsonString::from(result_raw);

            Ok((id, RpcResponse::Success { result }))
        }
    }
}

/// JSON-RPC 2.0 メッセージを組み立てる。
///
/// notification の場合は id を付与しない。
/// 戻り値は (メッセージ文字列, id) のタプル。
pub(crate) fn build_rpc_message(
    id_counter: &mut u64,
    method: &str,
    params: Option<&JsonString>,
    notification: bool,
) -> (String, Option<u64>) {
    let id = if notification {
        None
    } else {
        *id_counter += 1;
        Some(*id_counter)
    };

    let message = nojson::object(|f| {
        f.member("jsonrpc", "2.0")?;
        f.member("method", method)?;
        if let Some(params) = params {
            f.member("params", params)?;
        }
        if let Some(id) = id {
            f.member("id", id)?;
        }
        Ok(())
    })
    .to_string();

    (message, id)
}
