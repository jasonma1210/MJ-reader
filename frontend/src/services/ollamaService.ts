// Ollama 专属配置服务（2026-09-04）：地址/模型持久化 + 连接测试 + 模型列表。
// 后端命令无 feature 门控，全平台可用。

import { CMD, invoke, isTauri } from "./tauri";

/** Ollama 配置（settings KV 持久化） */
export interface OllamaConfig {
  baseUrl: string;
  /** 默认模型（/api/tags 的 name），空串表示未选择 */
  model: string;
}

/** 连接测试结果 */
export interface OllamaTestResult {
  ok: boolean;
  /** 模型名列表（字母序） */
  models: string[];
  latencyMs: number;
  error: string | null;
}

function guard(): void {
  if (!isTauri()) {
    throw new Error("Tauri 环境不可用");
  }
}

/** 读配置（无记录返回默认 http://localhost:11434 + 空模型） */
export function ollamaLoadConfig(): Promise<OllamaConfig> {
  guard();
  return invoke<OllamaConfig>(CMD.ollamaLoadConfig);
}

/** 保存配置 */
export function ollamaSaveConfig(baseUrl: string, model: string): Promise<void> {
  guard();
  return invoke<void>(CMD.ollamaSaveConfig, { baseUrl, model });
}

/** 测试连接并拉取模型列表（GET {base}/api/tags，5s 超时） */
export function ollamaTestConnection(baseUrl: string): Promise<OllamaTestResult> {
  guard();
  return invoke<OllamaTestResult>(CMD.ollamaTestConnection, { baseUrl });
}
