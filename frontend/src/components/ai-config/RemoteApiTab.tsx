import { AIModelConfig } from "../me/AIModelConfig";

/**
 * 远程模型 Tab（spec #5，2026-08-15）：
 * 直接渲染完整远程模型档案管理（AIModelConfig），支持配置「多个」远程模型，
 * 不内置任何默认 provider / baseUrl / 模型名——所有字段由用户自行填写。
 * 单一启用通过 AIModelConfig 的 isPrimary 单选实现。
 *
 * 2026-09-04 三源互斥：locked=true（端侧推理 / Ollama 生效中）时，
 * 所有启用开关强制呈关闭态且不可操作（配置保留，切回远程 API 自动恢复）。
 */
export function RemoteApiTab({ locked = false }: { locked?: boolean }) {
  return <AIModelConfig locked={locked} />;
}
