import { useSyncExternalStore, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ConfirmDialog } from "./ConfirmDialog";

/**
 * 命令式确认（原生 window.confirm 的替代，V2 S4 弹窗清零）：
 * - 组件侧：在壳层挂载唯一 <ConfirmHost />；
 * - 业务侧：`if (!(await askConfirm(t("xxx.deleteConfirm", { name })))) return;`
 *   与原生 confirm 同构（同步等待布尔），迁移零逻辑改动。
 */
interface ConfirmRequest {
  message?: string;
  title?: string;
  confirmText?: string;
  danger?: boolean;
  resolve: (ok: boolean) => void;
}

let current: ConfirmRequest | null = null;
const listeners = new Set<() => void>();

/** 弹出确认框并等待用户选择；遮罩/取消 = false。 */
export function askConfirm(
  message?: string,
  opts?: { title?: string; confirmText?: string; danger?: boolean },
): Promise<boolean> {
  return new Promise((resolve) => {
    // 已有未决请求时，前一个按取消处理（理论不可达：单弹窗交互）
    if (current) current.resolve(false);
    current = { message, ...opts, resolve };
    listeners.forEach((l) => l());
  });
}

function settle(ok: boolean) {
  if (!current) return;
  const { resolve } = current;
  current = null;
  listeners.forEach((l) => l());
  resolve(ok);
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/** 全局确认弹窗宿主：挂在壳层根部，渲染当前 pending 的确认请求。 */
export function ConfirmHost(): ReactNode {
  const req = useSyncExternalStore(
    subscribe,
    () => current,
    () => null,
  );
  const { t } = useTranslation();
  if (!req) return null;
  return (
    <ConfirmDialog
      open
      message={req.message}
      title={req.title}
      confirmText={req.confirmText}
      danger={req.danger}
      onConfirm={() => settle(true)}
      onCancel={() => settle(false)}
      cancelText={t("common.cancel")}
    />
  );
}
