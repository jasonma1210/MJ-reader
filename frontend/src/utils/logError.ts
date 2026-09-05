/**
 * 统一错误日志：所有空 catch / 异步失败分支调用，避免静默吞错。
 * Console 输出用英文（i18n 棘轮要求 UI 文案零中文，日志不受限）。
 */
export function logError(context: string, error: unknown): void {
  const detail = error instanceof Error ? error.stack ?? error.message : String(error);
  // eslint-disable-next-line no-console
  console.error(`[${context}]`, detail);
}
