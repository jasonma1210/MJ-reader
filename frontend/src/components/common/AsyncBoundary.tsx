import type { ReactNode } from "react";
import { LoadingState } from "./states/LoadingState";
import { EmptyState } from "./states/EmptyState";
import { ErrorState } from "./states/ErrorState";
import type { AsyncState } from "../../hooks/useAsyncState";

interface AsyncBoundaryProps<T> {
  /** useAsyncState 的返回值 */
  state: AsyncState<T>;
  /** 数据为空的判定（如数组 length === 0）；不传则仅判 null */
  isEmpty?: (data: T) => boolean;
  /** 自定义空态 / 加载态 / 错误态节点（缺省用统一三件套） */
  empty?: ReactNode;
  loading?: ReactNode;
  /** 透传给 ErrorState 的重试按钮文案 key */
  retryLabel?: string;
  /** success 且非空时的渲染函数 */
  children: (data: T) => ReactNode;
}

/**
 * 异步渲染边界（S1 §2.2 状态底座）：
 * loading → LoadingState；error → ErrorState（fail-closed，带重试）；
 * 空 → EmptyState；正常 → children(data)。
 * 配合 useAsyncState 使用，让页面只需声明「成功态长什么样」。
 */
export function AsyncBoundary<T>({
  state,
  isEmpty,
  empty,
  loading,
  retryLabel,
  children,
}: AsyncBoundaryProps<T>) {
  if (state.status === "loading") {
    return <>{loading ?? <LoadingState />}</>;
  }
  if (state.status === "error") {
    return (
      <ErrorState message={state.error ?? ""} onRetry={state.reload} retryLabel={retryLabel} />
    );
  }
  const data = state.data;
  if (data === null || data === undefined || (isEmpty && isEmpty(data))) {
    return <>{empty ?? <EmptyState />}</>;
  }
  return <>{children(data)}</>;
}
