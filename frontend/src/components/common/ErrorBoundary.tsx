import { Component, type ReactNode } from "react";
import { logError } from "../../utils/logError";

interface ErrorBoundaryProps {
  children: ReactNode;
  /** 自定义降级 UI（建议本地化）；缺省显示极简英文占位 */
  fallback?: ReactNode;
  /** 崩溃回调，便于上报真实错误栈用于定位 */
  onError?: (error: Error) => void;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error?: Error;
}

/**
 * 通用错误边界：隔离子树渲染期崩溃，避免单点组件异常冒泡卸载整页
 * （例如工作区面板崩溃连累阅读器整屏消失）。
 * 仅捕获渲染期/生命周期同步错误，不捕获事件回调与异步错误。
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { hasError: false };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: unknown): void {
    logError("ErrorBoundary", `${error.message}\n${(info as { componentStack?: string })?.componentStack ?? ""}`);
    this.props.onError?.(error);
  }

  render(): ReactNode {
    if (this.state.hasError) {
      return (
        this.props.fallback ?? (
          <div className="p-4 text-sm text-ink-muted">Something went wrong. Please retry.</div>
        )
      );
    }
    return this.props.children;
  }
}
