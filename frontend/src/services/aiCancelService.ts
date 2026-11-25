/**
 * AI 请求统一取消（W2）：AbortController 注册表 + Rust 取消命令。
 *
 * 前端 abort 只断开 JS 侧的等待；正在进行的远程 HTTP / 端侧推理必须靠 Rust
 * 侧 ai_cancel_request 唤醒 CancellationToken 才能真实停止。两者成对调用。
 *
 * 作用域键约定（与 Rust services/ai_cancel.rs 一致）：
 *   chat:<conversationId> —— 单次对话/问答
 *   book:<bookId>        —— 切书时一键终止该书全部请求
 */
import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

interface ActiveRequest {
  controller: AbortController;
  bookId: string | null;
}

const active = new Map<string, ActiveRequest>();

function fireRustCancel(requestId: string): void {
  if (!isTauri()) return;
  invoke<void>(CMD.aiCancelRequest, { requestId }).catch((e) => {
    logError("aiCancelService.fireRustCancel", e);
  });
}

/** 注册一个进行中的请求；同键重复注册会先取消旧请求。 */
export function registerAiRequest(key: string, bookId: string | null): AbortController {
  if (active.has(key)) cancelAiRequest(key);
  const controller = new AbortController();
  active.set(key, { controller, bookId });
  return controller;
}

/** 按 chat:<id> / book:<id> 取消；abort JS 等待 + 通知 Rust 终止请求。 */
export function cancelAiRequest(key: string): boolean {
  const entry = active.get(key);
  if (!entry) return false;
  active.delete(key);
  entry.controller.abort();
  fireRustCancel(key);
  return true;
}

/** 请求正常结束后的清理：只注销，不再发取消 IPC。 */
export function releaseAiRequest(key: string): void {
  active.delete(key);
}

/** 切书/退出会话：终止指定书籍的全部请求（Rust 侧同名 book: 令牌已挂同一 token）。 */
export function cancelAiRequestsByBook(bookId: string): number {
  let count = 0;
  for (const [key, entry] of [...active]) {
    if (entry.bookId === bookId) {
      active.delete(key);
      entry.controller.abort();
      count += 1;
    }
  }
  if (count > 0) fireRustCancel(`book:${bookId}`);
  return count;
}

/** 退出应用/全局重置：终止所有在途 AI 请求。 */
export function cancelAllAiRequests(): void {
  const keys = [...active.keys()];
  for (const key of keys) cancelAiRequest(key);
}

/** 仅测试使用：当前注册数量。 */
export function activeAiRequestCount(): number {
  return active.size;
}
