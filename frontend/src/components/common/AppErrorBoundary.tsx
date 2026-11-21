import { Component, type ErrorInfo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

const ONBOARDED_KEY = "mjnexus.onboarded";

interface AppErrorBoundaryProps {
  children: ReactNode;
  /** 崩溃来源，写入 console 便于定位（如 "app" / "onboarding"） */
  source?: string;
}

interface AppErrorBoundaryState {
  error: Error | null;
  componentStack: string;
}

interface ErrorFallbackProps {
  error: Error | null;
  componentStack: string;
  source?: string;
}

/** 可操作的错误态：重试 / 只清引导标记后重试；DEV 下展示错误栈 */
function ErrorFallback({ error, componentStack, source }: ErrorFallbackProps) {
  const { t } = useTranslation();

  const reload = () => window.location.reload();
  const clearOnboardingAndReload = () => {
    // 只清首启引导标记，不碰其他任何用户数据
    localStorage.removeItem(ONBOARDED_KEY);
    window.location.reload();
  };

  return (
    <div
      role="alert"
      className="fixed inset-0 z-[100] flex flex-col items-center justify-center gap-5 bg-paper px-6 text-center text-ink"
    >
      <div className="w-full max-w-md">
        <h1 className="text-lg font-bold">{t("errorBoundary.title")}</h1>
        <p className="mt-2 text-sm leading-relaxed text-ink-muted">
          {t("errorBoundary.desc")}
        </p>
        {import.meta.env.DEV && (
          <pre
            data-testid="error-boundary-stack"
            className="mt-4 max-h-48 overflow-auto rounded-lg bg-paper-soft p-3 text-left text-xs text-ink-muted"
          >
            {`[${source ?? "app"}] ${error?.stack ?? error?.message ?? ""}\n${componentStack}`}
          </pre>
        )}
      </div>
      <div className="flex flex-col gap-3 sm:flex-row">
        <button
          type="button"
          onClick={reload}
          className="rounded-full bg-accent px-6 py-2.5 text-sm font-medium text-accent-fg transition active:opacity-80"
        >
          {t("errorBoundary.retry")}
        </button>
        <button
          type="button"
          onClick={clearOnboardingAndReload}
          className="rounded-full border border-line px-6 py-2.5 text-sm font-medium text-ink transition active:opacity-80"
        >
          {t("errorBoundary.retryClearOnboarding")}
        </button>
      </div>
    </div>
  );
}

/**
 * App 顶层错误边界（P0-A 首启白屏兜底）：
 * 隔离渲染期 / 生命周期同步崩溃，显示可恢复的错误态而不是白屏。
 * 真崩溃只显示 fallback，绝不吞错继续渲染 children。
 */
export class AppErrorBoundary extends Component<
  AppErrorBoundaryProps,
  AppErrorBoundaryState
> {
  state: AppErrorBoundaryState = { error: null, componentStack: "" };

  static getDerivedStateFromError(error: Error): Partial<AppErrorBoundaryState> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    this.setState({ componentStack: info.componentStack ?? "" });
    const tag = this.props.source
      ? `AppErrorBoundary:${this.props.source}`
      : "AppErrorBoundary";
    // eslint-disable-next-line no-console
    console.error(`[${tag}]`, error, info.componentStack ?? "");
  }

  render(): ReactNode {
    if (this.state.error) {
      return (
        <ErrorFallback
          error={this.state.error}
          componentStack={this.state.componentStack}
          source={this.props.source}
        />
      );
    }
    return this.props.children;
  }
}
