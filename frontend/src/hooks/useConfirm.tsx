import { useCallback, useState } from "react";
import { ConfirmDialog } from "../components/common/ConfirmDialog";

interface ConfirmOptions {
  title?: string;
  confirmText?: string;
}

interface ConfirmState {
  message: string;
  title?: string;
  confirmText?: string;
  resolve: (value: boolean) => void;
}

/**
 * 应用内确认弹窗 Hook（替代 window.confirm）。
 *
 * 用法：
 *   const { confirm, dialog } = useConfirm();
 *   const onDelete = async () => {
 *     if (!(await confirm(t("x.confirmDelete")))) return;
 *     ...执行删除...
 *   };
 *   return (<><ExistingUI />{dialog}</>);
 *
 * confirm() 返回 Promise<boolean>，用户点「确定」为 true、「取消」为 false，
 * 在所有平台表现一致（修复 Tauri Android window.confirm 静默返回 false 的问题）。
 */
export function useConfirm() {
  const [state, setState] = useState<ConfirmState | null>(null);

  const confirm = useCallback(
    (message: string, opts?: ConfirmOptions) =>
      new Promise<boolean>((resolve) => {
        setState({
          message,
          title: opts?.title,
          confirmText: opts?.confirmText,
          resolve,
        });
      }),
    [],
  );

  const close = (value: boolean) => {
    const resolver = state?.resolve;
    setState(null);
    resolver?.(value);
  };

  const dialog = state ? (
    <ConfirmDialog
      open
      title={state.title}
      message={state.message}
      confirmText={state.confirmText}
      onConfirm={() => close(true)}
      onCancel={() => close(false)}
    />
  ) : null;

  return { confirm, dialog };
}
