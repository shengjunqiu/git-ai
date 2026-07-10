use crate::api::client::ApiClient;
use crate::api::types::{ApiErrorResponse, CreateBundleRequest, CreateBundleResponse};
use crate::error::GitAiError;

// ddd

/// Bundle API 端点实现
///
/// 该模块为 `ApiClient` 实现与 bundle（代码包）相关的 API 调用方法。
/// bundle 用于将一组 prompts 和关联的文件变更打包提交到服务端。
impl ApiClient {
    /// 创建新的 bundle，通过 POST 请求发送到 `/api/bundles`
    ///
    /// 该接口用于将本地的 prompt 记录和代码变更打包上传到服务端，
    /// 以便分享、审查或存档。服务端返回 bundle 的唯一 ID 和访问 URL。
    ///
    /// # 请求流程
    /// 1. 将 `CreateBundleRequest` 序列化为 JSON，POST 到 `/api/bundles`
    /// 2. 根据 HTTP 状态码分别处理成功和各类错误场景
    /// 3. 对于 API 层面的错误（400/500），尝试解析服务端返回的 `ApiErrorResponse`，
    ///    解析失败时回退到默认错误信息
    ///
    /// # 参数
    /// * `request` - bundle 创建请求，包含标题（title）、prompts 数据（data.prompts）
    ///   以及可选的文件变更记录（data.files）
    ///
    /// # 返回值
    /// * `Ok(CreateBundleResponse)` - 成功，包含 `id`（bundle 唯一标识）和 `url`（访问链接）
    /// * `Err(GitAiError)` - 失败
    ///
    /// # 错误类型
    /// * `GitAiError::Generic` - HTTP 请求本身失败（网络错误等）或 API 返回非 200 的业务错误
    /// * `GitAiError::JsonError` - 成功响应（200）的 JSON 反序列化失败
    pub fn create_bundle(
        &self,
        request: CreateBundleRequest,
    ) -> Result<CreateBundleResponse, GitAiError> {
        // 发起 POST 请求，将请求体序列化为 JSON 发送
        let response = self.context().post_json("/api/bundles", &request)?;
        let status_code = response.status_code;

        // 读取响应体文本，失败时包装为 Generic 错误
        let body = response
            .as_str()
            .map_err(|e| GitAiError::Generic(format!("Failed to read response body: {}", e)))?;

        // 根据 HTTP 状态码分派不同的处理逻辑
        match status_code {
            // 200: 请求成功，反序列化 bundle 创建响应
            200 => {
                let bundle_response: CreateBundleResponse =
                    serde_json::from_str(body).map_err(GitAiError::JsonError)?;
                Ok(bundle_response)
            }
            // 400: 客户端请求错误（如缺少必填字段、数据格式错误等）
            // 尝试解析服务端结构化错误信息，解析失败时提供默认错误描述
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
            // 500: 服务端内部错误，同样尝试解析结构化的错误响应
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
            // 其他未知状态码：透传原始状态码和响应体，方便排查
            _ => Err(GitAiError::Generic(format!(
                "Unexpected status code {}: {}",
                status_code, body
            ))),
        }
    }
}
