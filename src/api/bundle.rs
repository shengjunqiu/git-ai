use crate::api::client::ApiClient;
use crate::api::types::{ApiErrorResponse, CreateBundleRequest, CreateBundleResponse};
use crate::error::GitAiError;


/// Bundle 相关的 API 操作。
impl ApiClient {
    /// 创建一个 bundle，并返回服务端生成的响应数据。
    ///
    /// 该方法会将请求体序列化为 JSON 后发送到 `/api/bundles`。
    pub fn create_bundle(
        &self,
        request: CreateBundleRequest,
    ) -> Result<CreateBundleResponse, GitAiError> {
        // 发起 POST 请求，将 bundle 请求数据发送到服务端。
        let response = self.context().post_json("/api/bundles", &request)?;
        let status_code = response.status_code;

        // 读取响应体字符串，用于后续 JSON 反序列化和错误处理。
        let body = response
            .as_str()
            .map_err(|e| GitAiError::Generic(format!("Failed to read response body: {}", e)))?;

        match status_code {
            200 => {
                // 成功状态时解析 bundle 响应结构。
                let bundle_response: CreateBundleResponse =
                    serde_json::from_str(body).map_err(GitAiError::JsonError)?;
                Ok(bundle_response)
            }
            400 => {
                // 请求格式或参数错误，尝试从响应中提取详细错误信息。
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
                // 服务端内部错误，返回通用错误消息。
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
            _ => Err(GitAiError::Generic(format!(
                // 其他未知状态码，保留原始响应便于诊断。
                "Unexpected status code {}: {}",
                status_code, body
            ))),
        }
    }
}
