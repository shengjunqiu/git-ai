use crate::api::client::ApiClient;
use crate::api::types::{ApiErrorResponse, CreateBundleRequest, CreateBundleResponse};
use crate::error::GitAiError;



/// Bundle 相关的 API 操作。
impl ApiClient {
    /// 将 prompt 记录及其关联的文件变更上传为一个 bundle。
    ///
    /// 请求成功时返回 bundle 的标识和访问信息。对于客户端或服务端错误，
    /// 优先使用响应体中的结构化错误消息；无法解析时使用默认描述。
    ///
    /// # Errors
    ///
    /// - 请求发送或响应体读取失败时返回 `GitAiError::Generic`。
    /// - 成功响应无法反序列化时返回 `GitAiError::JsonError`。
    /// - API 返回非成功状态码时返回包含服务端错误信息的 `GitAiError::Generic`。
    pub fn create_bundle(
        &self,
        request: CreateBundleRequest,
    ) -> Result<CreateBundleResponse, GitAiError> {
        // 请求上下文负责将请求体序列化为 JSON。
        let response = self.context().post_json("/api/bundles", &request)?;
        let status_code = response.status_code;

        let body = response
            .as_str()
            .map_err(|e| GitAiError::Generic(format!("Failed to read response body: {}", e)))?;

        match status_code {
            // 成功响应必须符合 bundle 创建响应结构。
            200 => {
                let bundle_response: CreateBundleResponse =
                    serde_json::from_str(body).map_err(GitAiError::JsonError)?;
                Ok(bundle_response)
            }
            // 对已知错误状态，尽可能保留服务端返回的错误原因。
            400 => {
                let error_response: ApiErrorResponse =
                    serde_json::from_str(body).unwrap_or_else(|_| ApiErrorResponse {
                        error: "Invalid request body".to_string(),
                        details: Some(serde_json::Value::String(body.to_string())),
                    });
                Err(GitAiError::Generic(format!(
                    "Bad Request: {}",
                    error_response.error
                )))
            }
            500 => {
                let error_response: ApiErrorResponse =
                    serde_json::from_str(body).unwrap_or_else(|_| ApiErrorResponse {
                        error: "Internal server error".to_string(),
                        details: None,
                    });
                Err(GitAiError::Generic(format!(
                    "Internal Server Error: {}",
                    error_response.error
                )))
            }
            // 未显式支持的状态码保留原始响应体，便于诊断协议变化。
            _ => Err(GitAiError::Generic(format!(
                "Unexpected status code {}: {}",
                status_code, body
            ))),
        }
    }
}
