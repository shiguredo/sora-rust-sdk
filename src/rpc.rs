//! JSON-RPC 2.0 over DataChannel の型と組み立て。
use std::time::Duration;

use nojson::{DisplayJson, JsonValueKind, RawJsonOwned};

use crate::error::{Error, Result};
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

        // top-level は単一の JSON Object でなければならない。
        // SDK は batch request を送信しないため、Array の batch response には対応しない。
        // (JSON-RPC 2.0 Specification Section 5 "Response object")
        if value.kind() != JsonValueKind::Object {
            return Err(protocol_violation(None));
        }

        // 制御 member の重複を検出する。
        // member 名は JSON escape を decode した後で比較し、"id" と "\u0069d" も重複として扱う。
        // unknown member の重複は JSON-RPC の解釈へ影響しないため拒否しない。
        let mut jsonrpc_seen = false;
        let mut jsonrpc_duplicated = false;
        let mut id_seen = false;
        let mut id_duplicated = false;
        let mut result_seen = false;
        let mut result_duplicated = false;
        let mut error_seen = false;
        let mut error_duplicated = false;
        for (key, _member_value) in value.to_object()? {
            let name = key.to_unquoted_string_str()?;
            match name.as_ref() {
                "jsonrpc" => {
                    if jsonrpc_seen {
                        jsonrpc_duplicated = true;
                    }
                    jsonrpc_seen = true;
                }
                "id" => {
                    if id_seen {
                        id_duplicated = true;
                    }
                    id_seen = true;
                }
                "result" => {
                    if result_seen {
                        result_duplicated = true;
                    }
                    result_seen = true;
                }
                "error" => {
                    if error_seen {
                        error_duplicated = true;
                    }
                    error_seen = true;
                }
                _ => {}
            }
        }

        // id を検査する。
        // JSON-RPC 2.0 の仕様上では文字列の id も可能だが、SDK 内部では u64 として扱っているので、
        // u64 に変換できない値は None として扱う
        let trusted_id = if id_duplicated {
            None
        } else {
            value
                .to_member("id")?
                .optional()
                .filter(|v| v.kind() != JsonValueKind::Null)
                .and_then(|v| v.try_into().ok())
        };

        // 重複する要素がある場合は protocol violation とする。
        // (RFC 8259 Section 4)
        if jsonrpc_duplicated || result_duplicated || error_duplicated {
            return Err(protocol_violation(trusted_id));
        }

        // jsonrpc member は必須で、String の "2.0" と大文字・小文字を含め完全一致する。
        // (JSON-RPC 2.0 Specification Section 5 "Response object")
        let jsonrpc_ok = value
            .to_member("jsonrpc")?
            .optional()
            .and_then(|v| v.to_unquoted_string_str().ok())
            .is_some_and(|s| s == "2.0");
        if !jsonrpc_ok {
            return Err(protocol_violation(trusted_id));
        }

        // result と error は member の存在で判定し、必ず一方だけが存在する。
        // result: null は member が存在する正常な success response であり、
        // error: null は Error Object ではないため protocol violation として扱う。
        // (JSON-RPC 2.0 Specification Section 5 "Response object")
        match (result_seen, error_seen) {
            (true, false) => {
                let result_value = value
                    .to_member("result")?
                    .required()
                    .expect("result_seen なので必ず存在する");
                let result =
                    JsonString::from(RawJsonOwned::json(|f| DisplayJson::fmt(&result_value, f)));
                Ok((trusted_id, RpcResponse::Success { result }))
            }
            (false, true) => {
                let error_value = value
                    .to_member("error")?
                    .required()
                    .expect("error_seen なので必ず存在する");
                let (code, message, data) = parse_error_object(trusted_id, error_value)?;
                Ok((
                    trusted_id,
                    RpcResponse::Error {
                        code,
                        message,
                        data,
                    },
                ))
            }
            _ => Err(protocol_violation(trusted_id)),
        }
    }
}

/// JSON-RPC 2.0 の要件を満たさない応答を表すエラーを生成する。
///
/// raw response と不正な field の実値を一切保持しない。
/// SDK が request と相関できた trusted id だけを保持する。
fn protocol_violation(id: Option<u64>) -> Error {
    Error::RpcProtocolViolation { id }
}

/// Response Object の `error` member を検証して解析する。
fn parse_error_object(
    trusted_id: Option<u64>,
    error_value: nojson::RawJsonValue<'_, '_>,
) -> Result<(i32, String, Option<JsonString>)> {
    // error は Error Object でなければならない。
    // (JSON-RPC 2.0 Specification Section 5.1 "Error object")
    if error_value.kind() != JsonValueKind::Object {
        return Err(protocol_violation(trusted_id));
    }

    // Error Object の制御 member の重複を検出する。
    // (RFC 8259 Section 4 "Objects"、JSON-RPC 2.0 Specification Section 5.1 "Error object")
    let mut code_seen = false;
    let mut message_seen = false;
    let mut data_seen = false;
    for (key, _member_value) in error_value.to_object()? {
        let name = key.to_unquoted_string_str()?;
        match name.as_ref() {
            "code" => {
                if code_seen {
                    return Err(protocol_violation(trusted_id));
                }
                code_seen = true;
            }
            "message" => {
                if message_seen {
                    return Err(protocol_violation(trusted_id));
                }
                message_seen = true;
            }
            "data" => {
                if data_seen {
                    return Err(protocol_violation(trusted_id));
                }
                data_seen = true;
            }
            _ => {}
        }
    }

    // code は必須で、小数点と指数表記を含まない字句上の JSON 整数かつ
    // 既存の公開 field (i32) の範囲内でなければならない。
    // i32 範囲外の整数と、数学的な値が整数でも小数点または指数表記を含む Number は、
    // SDK が表現または厳密に判定できないため拒否する。
    // 浮動小数点数への変換で丸めて整数とみなさない。
    // (JSON-RPC 2.0 Specification Section 5.1 "Error object")
    let code_member = error_value.to_member("code")?;
    let code_value = match code_member.required() {
        Ok(value) => value,
        Err(_) => return Err(protocol_violation(trusted_id)),
    };
    let code: i32 = match code_value.try_into() {
        Ok(code) => code,
        Err(_) => return Err(protocol_violation(trusted_id)),
    };

    // message は必須の String でなければならない。
    // (JSON-RPC 2.0 Specification Section 5.1 "Error object")
    let message_member = error_value.to_member("message")?;
    let message_value = match message_member.required() {
        Ok(value) => value,
        Err(_) => return Err(protocol_violation(trusted_id)),
    };
    let message: String = match message_value.try_into() {
        Ok(message) => message,
        Err(_) => return Err(protocol_violation(trusted_id)),
    };

    // data は省略可能で、存在する場合は null を含む任意の JSON 値を保持する。
    // (JSON-RPC 2.0 Specification Section 5.1 "Error object")
    let data: Option<JsonString> = error_value
        .to_member("data")?
        .optional()
        .map(|v| RawJsonOwned::json(|f| DisplayJson::fmt(&v, f)))
        .map(JsonString::from);

    Ok((code, message, data))
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

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    /// parse の期待結果。
    #[derive(Debug)]
    enum Expected {
        /// 正常 success response。
        Success {
            /// 信頼できる id。
            id: Option<u64>,
            /// result の compact な JSON 表記。
            result: &'static str,
        },
        /// 正常 remote error response。
        Error {
            /// 信頼できる id。
            id: Option<u64>,
            /// code。
            code: i32,
            /// message。
            message: &'static str,
            /// data。欠落は `None`、存在する場合は compact な JSON 表記。
            data: Option<&'static str>,
        },
        /// JSON-RPC 2.0 の要件を満たさない response。
        ProtocolViolation {
            /// 保持される trusted id。
            id: Option<u64>,
        },
        /// JSON syntax error。
        SyntaxError,
    }

    /// parse の結果が期待どおりであることを確認する。
    fn assert_parse(input: &str, expected: Expected) {
        let result = RpcResponse::parse(input);
        match expected {
            Expected::Success {
                id,
                result: expected_result,
            } => {
                let (actual_id, response) =
                    result.expect("Success を期待しましたが parse が失敗しました");
                assert_eq!(actual_id, id, "id が一致しません");
                let RpcResponse::Success { result } = response else {
                    panic!("Success を期待しましたが別の response になりました");
                };
                assert_eq!(
                    result.to_string(),
                    expected_result,
                    "result の JSON 表記が一致しません"
                );
            }
            Expected::Error {
                id,
                code,
                message,
                data,
            } => {
                let (actual_id, response) =
                    result.expect("Error を期待しましたが parse が失敗しました");
                assert_eq!(actual_id, id, "id が一致しません");
                let RpcResponse::Error {
                    code: actual_code,
                    message: actual_message,
                    data: actual_data,
                } = response
                else {
                    panic!("Error を期待しましたが別の response になりました");
                };
                assert_eq!(actual_code, code, "code が一致しません");
                assert_eq!(actual_message, message, "message が一致しません");
                assert_eq!(
                    actual_data.map(|d| d.to_string()),
                    data.map(str::to_string),
                    "data が一致しません"
                );
            }
            Expected::ProtocolViolation { id } => {
                let err =
                    result.expect_err("ProtocolViolation を期待しましたが parse が成功しました");
                match err {
                    Error::RpcProtocolViolation { id: actual_id } => {
                        assert_eq!(actual_id, id, "trusted id が一致しません");
                    }
                    other => {
                        panic!(
                            "ProtocolViolation を期待しましたが別のエラーになりました: {other:?}"
                        );
                    }
                }
            }
            Expected::SyntaxError => {
                let err = result.expect_err("SyntaxError を期待しましたが parse が成功しました");
                assert!(
                    matches!(err, Error::JsonParse(_)),
                    "JsonParse を期待しましたが別のエラーになりました: {err:?}"
                );
            }
        }
    }

    #[test]
    fn success_with_various_result_values() {
        // 正常 success response は、result が Object / Array / String / Number / Boolean / Null の
        // どの JSON 値でも受理される必要があります。
        // (JSON-RPC 2.0 Specification Section 5)
        for (input, result) in [
            (r#"{"jsonrpc":"2.0","id":1,"result":{"a":1}}"#, r#"{"a":1}"#),
            (r#"{"jsonrpc":"2.0","id":1,"result":[1,2,3]}"#, r#"[1,2,3]"#),
            (r#"{"jsonrpc":"2.0","id":1,"result":"text"}"#, r#""text""#),
            (r#"{"jsonrpc":"2.0","id":1,"result":42}"#, "42"),
            (r#"{"jsonrpc":"2.0","id":1,"result":true}"#, "true"),
            (r#"{"jsonrpc":"2.0","id":1,"result":null}"#, "null"),
        ] {
            assert_parse(
                input,
                Expected::Success {
                    id: Some(1),
                    result,
                },
            );
        }
    }

    #[test]
    fn success_with_zero_and_u64_max_id() {
        // id は 0 と u64::MAX を信頼できる id として受理する必要があります。
        assert_parse(
            r#"{"jsonrpc":"2.0","id":0,"result":null}"#,
            Expected::Success {
                id: Some(0),
                result: "null",
            },
        );
        assert_parse(
            r#"{"jsonrpc":"2.0","id":18446744073709551615,"result":null}"#,
            Expected::Success {
                id: Some(u64::MAX),
                result: "null",
            },
        );
    }

    #[test]
    fn remote_error_with_various_data_values() {
        // 正常 remote error response は、data が欠落 / Null / Primitive / Object / Array の
        // どの形でも受理され、message と data を保持する必要があります。
        // (JSON-RPC 2.0 Specification Section 5.1)
        for (input, data) in [
            (
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"m"}}"#,
                None,
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"m","data":null}}"#,
                Some("null"),
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"m","data":42}}"#,
                Some("42"),
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"m","data":{"k":1}}}"#,
                Some(r#"{"k":1}"#),
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"m","data":[1]}}"#,
                Some("[1]"),
            ),
        ] {
            assert_parse(
                input,
                Expected::Error {
                    id: Some(1),
                    code: -32000,
                    message: "m",
                    data,
                },
            );
        }
    }

    #[test]
    fn remote_error_with_code_boundaries() {
        // code は i32::MIN と i32::MAX を受理する必要があります。
        assert_parse(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-2147483648,"message":"m"}}"#,
            Expected::Error {
                id: Some(1),
                code: i32::MIN,
                message: "m",
                data: None,
            },
        );
        assert_parse(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":2147483647,"message":"m"}}"#,
            Expected::Error {
                id: Some(1),
                code: i32::MAX,
                message: "m",
                data: None,
            },
        );
    }

    #[test]
    fn invalid_jsonrpc_is_protocol_violation() {
        // jsonrpc member は必須で、String の "2.0" と大文字・小文字を含め完全一致する必要があります。
        // "2.0" にはアルファベットが含まれないため、完全一致は
        // 欠落 / Null / Number / "2.0" 以外の String / 前後空白付き String で検証します。
        // (JSON-RPC 2.0 Specification Section 5)
        for input in [
            r#"{"id":1,"result":null}"#,
            r#"{"jsonrpc":null,"id":1,"result":null}"#,
            r#"{"jsonrpc":2.0,"id":1,"result":null}"#,
            r#"{"jsonrpc":"2.1","id":1,"result":null}"#,
            r#"{"jsonrpc":"2.0 ","id":1,"result":null}"#,
            r#"{"jsonrpc":" 2.0","id":1,"result":null}"#,
            r#"{"jsonrpc":"2.00","id":1,"result":null}"#,
        ] {
            assert_parse(input, Expected::ProtocolViolation { id: Some(1) });
        }
    }

    #[test]
    fn untrustworthy_id_is_kept_as_none() {
        // 欠落 / Null / String / 負数 / 小数 / 指数表記 / u64 範囲外の id は
        // JSON-RPC 一般の ID 型を否定するものではなく、本 SDK が生成した Request ID と
        // 相関できない値として扱うため、それ以外の validation が成功していれば
        // id: None の成功として返します。
        for input in [
            r#"{"jsonrpc":"2.0","result":null}"#,
            r#"{"jsonrpc":"2.0","id":null,"result":null}"#,
            r#"{"jsonrpc":"2.0","id":"1","result":null}"#,
            r#"{"jsonrpc":"2.0","id":-1,"result":null}"#,
            r#"{"jsonrpc":"2.0","id":1.5,"result":null}"#,
            r#"{"jsonrpc":"2.0","id":1e2,"result":null}"#,
            r#"{"jsonrpc":"2.0","id":18446744073709551616,"result":null}"#,
        ] {
            assert_parse(
                input,
                Expected::Success {
                    id: None,
                    result: "null",
                },
            );
        }
    }

    #[test]
    fn result_and_error_must_be_exclusive() {
        // result と error は member の存在で判定し、必ず一方だけが存在する必要があります。
        // 両方欠落と両方存在は protocol violation です。
        // (JSON-RPC 2.0 Specification Section 5)
        for input in [
            r#"{"jsonrpc":"2.0","id":1}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":null,"error":{"code":1,"message":"m"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":null}"#,
        ] {
            assert_parse(input, Expected::ProtocolViolation { id: Some(1) });
        }
        // result: null は member が存在する正常な success response です。
        assert_parse(
            r#"{"jsonrpc":"2.0","id":1,"result":null}"#,
            Expected::Success {
                id: Some(1),
                result: "null",
            },
        );
    }

    #[test]
    fn invalid_error_object_is_protocol_violation() {
        // error が Object ではない場合と、Error Object の code / message が不正な場合は
        // protocol violation です。
        // (JSON-RPC 2.0 Specification Section 5.1)
        for input in [
            r#"{"jsonrpc":"2.0","id":1,"error":[1]}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"message":"m"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":"1","message":"m"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":1.5,"message":"m"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":1e3,"message":"m"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":2147483648,"message":"m"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-2147483649,"message":"m"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":1}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":1,"message":42}}"#,
        ] {
            assert_parse(input, Expected::ProtocolViolation { id: Some(1) });
        }
    }

    #[test]
    fn non_object_top_level_is_protocol_violation() {
        // top-level は単一の JSON Object である必要があります。
        // SDK は batch request を送信しないため、Array の batch response は受理しません。
        // (JSON-RPC 2.0 Specification Section 5)
        for input in [
            "null",
            "true",
            "42",
            r#""string""#,
            r#"[{"jsonrpc":"2.0","id":1,"result":null}]"#,
        ] {
            assert_parse(input, Expected::ProtocolViolation { id: None });
        }
    }

    #[test]
    fn additional_member_and_arbitrary_order_are_accepted() {
        // JSON-RPC が定義しない追加 member と任意の member 順序は受理結果へ影響させません。
        assert_parse(
            r#"{"extra":1,"result":null,"id":1,"jsonrpc":"2.0","more":[1]}"#,
            Expected::Success {
                id: Some(1),
                result: "null",
            },
        );
        // unknown member の重複は JSON-RPC の解釈へ影響しないため拒否しません。
        // (RFC 8259 Section 4)
        assert_parse(
            r#"{"extra":1,"extra":2,"result":null,"id":1,"jsonrpc":"2.0"}"#,
            Expected::Success {
                id: Some(1),
                result: "null",
            },
        );
    }

    #[test]
    fn duplicated_control_member_is_protocol_violation() {
        // response Object の jsonrpc / result / error が重複していたら protocol violation です。
        // id は独立に検査されるため、id が重複していなければ trusted id は保持されます。
        // (RFC 8259 Section 4)
        for input in [
            r#"{"jsonrpc":"2.0","jsonrpc":"2.0","id":1,"result":null}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":null,"result":null}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":1,"message":"m"},"error":{"code":1,"message":"m"}}"#,
        ] {
            assert_parse(input, Expected::ProtocolViolation { id: Some(1) });
        }
        // id が欠落している場合は trusted id も None です。
        assert_parse(
            r#"{"jsonrpc":"2.0","result":null,"result":null}"#,
            Expected::ProtocolViolation { id: None },
        );
    }

    #[test]
    fn duplicated_id_member_loses_trusted_id() {
        // id member 自体が重複している場合は、値が同一でも相関が曖昧なため
        // 信頼できる id を確立しません。
        assert_parse(
            r#"{"jsonrpc":"2.0","id":1,"id":1,"result":null}"#,
            Expected::Success {
                id: None,
                result: "null",
            },
        );
        assert_parse(
            r#"{"jsonrpc":"2.0x","id":1,"id":1,"result":null}"#,
            Expected::ProtocolViolation { id: None },
        );
    }

    #[test]
    fn escaped_member_name_is_compared_after_decode() {
        // member 名は JSON escape を decode した後で比較するため、
        // raw 表記が異なっても "id" と "\u0069d" は重複として扱います。
        // (RFC 8259 Section 4)
        assert_parse(
            r#"{"jsonrpc":"2.0","id":1,"\u0069d":1,"result":null}"#,
            Expected::Success {
                id: None,
                result: "null",
            },
        );
        assert_parse(
            r#"{"jsonrpc":"2.0","id":1,"\u0072esult":null,"result":null}"#,
            Expected::ProtocolViolation { id: Some(1) },
        );
        assert_parse(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":1,"\u0063ode":1,"message":"m"}}"#,
            Expected::ProtocolViolation { id: Some(1) },
        );
    }

    #[test]
    fn valid_id_is_preserved_in_protocol_violation() {
        // response 全体の validation より先に id を独立に検査するため、
        // 有効な u64 id と不正 version の組み合わせは同じ trusted id を持つ violation になります。
        assert_parse(
            r#"{"jsonrpc":"2.0x","id":7,"result":null}"#,
            Expected::ProtocolViolation { id: Some(7) },
        );
        // 信頼できない id の場合、後続の validation が不正でも id は None です。
        assert_parse(
            r#"{"jsonrpc":"2.0x","result":null}"#,
            Expected::ProtocolViolation { id: None },
        );
    }

    #[test]
    fn json_syntax_error_is_json_parse() {
        for input in ["not json", r#"{"jsonrpc":"2.0","id":1,}"#] {
            assert_parse(input, Expected::SyntaxError);
        }
    }

    #[test]
    fn protocol_violation_error_does_not_leak_raw_response() {
        // validation error の Display / Debug / source() に、
        // field のダミー marker と raw response が含まれない必要があります。
        let marker = "DUMMY_MARKER";
        let raw_response = format!(r#"{{"jsonrpc":"{marker}","id":7,"result":null}}"#);
        let result = RpcResponse::parse(&raw_response);
        let err = match result {
            Err(err) => err,
            _ => panic!("ProtocolViolation を期待しましたが parse が成功しました"),
        };
        assert!(
            matches!(err, Error::RpcProtocolViolation { id: Some(7) }),
            "trusted id を保持した ProtocolViolation を期待しました: {err:?}"
        );
        let display = format!("{err}");
        let debug = format!("{err:?}");
        let source = err.source().map(|s| format!("{s}"));
        assert!(
            !display.contains(marker)
                && !debug.contains(marker)
                && !source.as_deref().is_some_and(|s| s.contains(marker)),
            "ダミー marker が error に漏れています: display={display} debug={debug} source={source:?}"
        );
        assert!(
            !display.contains(&raw_response)
                && !debug.contains(&raw_response)
                && !source.as_deref().is_some_and(|s| s.contains(&raw_response)),
            "raw response が error に漏れています: display={display} debug={debug} source={source:?}"
        );
    }
}
