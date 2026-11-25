/**
 * 后端并行实现期间的 CMD 键适配层（W3 云端 AI / W5 导出与反向链接）。
 *
 * 约定：命令名字面量只允许存在于 services/tauri.ts 的 CMD 注册表。
 * 本模块不写任何裸命令字符串，只声明「后端待补的键名」与注册表形状；
 * 后端工程师把键补进 CMD 后，本模块自动读取到真实命令名，无需改动调用方。
 * 未补齐时 hasPendingCmd() 返回 false，调用方显示「后端尚未接入」而非假数据。
 */
import { CMD, invoke as tauriInvoke, isTauri } from "../services/tauri";

export type PendingCmdKey =
  | "exportCsv"
  | "exportHtml"
  | "exportEpub"
  | "exportPdf"
  | "listBacklinksByHighlight"
  | "getAiUsage"
  | "getAiTaskRouting"
  | "saveAiTaskRouting"
  // 白板卡 → 知识图谱同步（后端待补键）
  | "syncWhiteboardCardToGraph";

type ExtendableCmd = typeof CMD & Partial<Record<PendingCmdKey, string>>;

const registry = CMD as ExtendableCmd;

/** CMD 键是否已由后端注册（且当前在 Tauri 运行时内）。 */
export function hasPendingCmd(key: PendingCmdKey): boolean {
  return typeof registry[key] === "string" && isTauri();
}

/**
 * 通过 CMD 注册表调用后端命令；键未注册或非 Tauri 运行时返回 null。
 * 不吞异常：调用方负责捕获并提示，便于定位后端错误。
 */
export async function invokePending<T>(
  key: PendingCmdKey,
  args?: Record<string, unknown>,
): Promise<T | null> {
  const cmd = registry[key];
  if (!cmd || !isTauri()) return null;
  return tauriInvoke<T>(cmd, args);
}
