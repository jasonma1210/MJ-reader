// v0.5.0 实现：MCP HTTP server + JSON-RPC 2.0 处理
// 监听 127.0.0.1:9124，POST /mcp 接收 JSON-RPC 2.0 请求
// 暴露书籍库/高亮/笔记等资源供外部 AI Agent 查询
//
// BE-07 修复（2026-08-05 审计）：此前无任何认证——同机任意进程（含浏览器网页配合
// DNS rebinding）都能读书库/标注/进度并写入标注，与 BE-06 组合成完整攻击链。
// 现强制校验随机 Bearer token（0600 权限文件持久化），并校验 Host 头。

use axum::{
    extract::{Request, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::net::TcpListener;

use super::resources;
use super::tools;

const MCP_PORT: u16 = 9124;
const MCP_SERVER_NAME: &str = "MJNexus-Reader";
const MCP_SERVER_VERSION: &str = "0.5.0";

#[derive(Clone)]
struct McpState {
    db: SqlitePool,
    /// BE-07：随机 bearer token（启动时生成/复用，写入 0600 权限文件）
    token: std::sync::Arc<String>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    // jsonrpc 字段为协议规范字段，此处仅做反序列化校验，运行时不读取
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

impl JsonRpcResponse {
    fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message }),
        }
    }
}

pub async fn start_mcp_server(
    db: SqlitePool,
    token: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = McpState {
        db,
        token: std::sync::Arc::new(token),
    };

    let app = Router::new()
        .route("/mcp", post(handle_mcp_request))
        .with_state(state);

    let addr = format!("127.0.0.1:{}", MCP_PORT);
    let listener = TcpListener::bind(&addr).await?;
    log::info!("[MCP] server listening on http://{} (bearer auth enabled)", addr);

    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle_mcp_request(
    State(state): State<McpState>,
    req: Request,
) -> Result<(StatusCode, Json<JsonRpcResponse>), (StatusCode, Json<JsonRpcResponse>)> {
    // BE-07：强制 Bearer token 认证——无 token / token 不匹配一律 401
    let auth_ok = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            let expected = format!("Bearer {}", state.token);
            v == expected
        });
    if !auth_ok {
        log::warn!("[MCP] 未认证请求被拒绝（缺少/错误的 Bearer token）");
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(JsonRpcResponse::error(
                None,
                -32001,
                "Unauthorized: missing or invalid Bearer token".to_string(),
            )),
        ));
    }

    // BE-07：校验 Host 头为 loopback（防 DNS rebinding / 伪造 Host 绕过）
    if let Some(host) = req.headers().get("host").and_then(|v| v.to_str().ok()) {
        let host_ok = host == "127.0.0.1:9124"
            || host == "localhost:9124"
            || host == "127.0.0.1"
            || host == "localhost";
        if !host_ok {
            log::warn!("[MCP] Host 头校验失败: {}", host);
            return Err((
                StatusCode::FORBIDDEN,
                Json(JsonRpcResponse::error(
                    None,
                    -32003,
                    "Forbidden: invalid Host header".to_string(),
                )),
            ));
        }
    }

    let body_bytes = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(JsonRpcResponse::error(
                    None,
                    -32700,
                    "Parse error: body too large or malformed".to_string(),
                )),
            ));
        }
    };
    let req: JsonRpcRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(JsonRpcResponse::error(
                    None,
                    -32700,
                    format!("Parse error: {}", e),
                )),
            ));
        }
    };

    log::info!(
        "[MCP] request: method={}, id={:?}",
        req.method,
        req.id
    );

    let response = match req.method.as_str() {
        "initialize" => handle_initialize(req.id),
        "resources/list" => handle_list_resources(req.id, &state.db).await,
        "resources/read" => handle_read_resource(req.id, req.params, &state.db).await,
        "tools/list" => handle_list_tools(req.id),
        "tools/call" => handle_call_tool(req.id, req.params, &state.db).await,
        _ => JsonRpcResponse::error(
            req.id,
            -32601,
            format!("Method not found: {}", req.method),
        ),
    };

    Ok((StatusCode::OK, Json(response)))
}

fn handle_initialize(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {
                "name": MCP_SERVER_NAME,
                "version": MCP_SERVER_VERSION
            },
            "capabilities": {
                "resources": { "list": true, "read": true },
                "tools": { "list": true, "call": true }
            }
        }),
    )
}

async fn handle_list_resources(id: Option<Value>, db: &SqlitePool) -> JsonRpcResponse {
    match resources::list_resources(db).await {
        Ok(res) => JsonRpcResponse::success(id, json!({ "resources": res })),
        Err(e) => JsonRpcResponse::error(id, -32603, format!("Internal error: {}", e)),
    }
}

async fn handle_read_resource(
    id: Option<Value>,
    params: Option<Value>,
    db: &SqlitePool,
) -> JsonRpcResponse {
    let uri = match params
        .as_ref()
        .and_then(|p| p.get("uri"))
        .and_then(|u| u.as_str())
    {
        Some(u) => u.to_string(),
        None => {
            return JsonRpcResponse::error(id, -32602, "Missing 'uri' parameter".to_string())
        }
    };

    match resources::read_resource(&uri, db).await {
        Ok(content) => JsonRpcResponse::success(
            id,
            json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": content
                }]
            }),
        ),
        Err(e) => JsonRpcResponse::error(id, -32603, format!("Read error: {}", e)),
    }
}

fn handle_list_tools(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse::success(id, json!({ "tools": tools::list_tools() }))
}

async fn handle_call_tool(
    id: Option<Value>,
    params: Option<Value>,
    db: &SqlitePool,
) -> JsonRpcResponse {
    let (name, arguments) = match params.as_ref().and_then(|p| {
        let name = p.get("name")?.as_str()?;
        let args = p.get("arguments").cloned().unwrap_or(Value::Null);
        Some((name.to_string(), args))
    }) {
        Some(x) => x,
        None => {
            return JsonRpcResponse::error(id, -32602, "Missing 'name' parameter".to_string())
        }
    };

    match tools::call_tool(&name, &arguments, db).await {
        Ok(result) => JsonRpcResponse::success(id, json!({ "content": result })),
        Err(e) => JsonRpcResponse::error(id, -32603, format!("Tool error: {}", e)),
    }
}
