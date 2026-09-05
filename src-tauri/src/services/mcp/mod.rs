// v0.5.0 实现：MCP (Model Context Protocol) 模块
// 让 MJNexus-Reader 成为 AI Agent 的知识源
// 通过 JSON-RPC 2.0 暴露书籍库、高亮、笔记等资源供外部 AI Agent 查询

pub mod resources;
pub mod server;
pub mod tools;
