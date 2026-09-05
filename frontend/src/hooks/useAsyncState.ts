import { useCallback, useEffect, useRef, useState } from "react";

/**
 * 异步加载状态 hook（S1 §2.2 状态底座）：
 * 统一 loading / success / error 三态，fail-closed——错误必须显式呈现。
 * 与 components/common/states/ 三件套（LoadingState/EmptyState/ErrorState）
 * 通过 AsyncBoundary 组合使用，替代各页散写的手工 setState 样板。
 *
 * 用法：
 *   const books = useAsyncState(() => bookService.listBooks(), [sortKey]);
 *   <AsyncBoundary state={books} isEmpty={(d) => d.length === 0}>
 *     {(data) => <BookList books={data} />}
 *   </AsyncBoundary>
 */

export type AsyncStatus = "loading" | "success" | "error";

export interface AsyncState<T> {
  data: T | null;
  status: AsyncStatus;
  /** 错误信息（success 态恒为 null） */
  error: string | null;
  /** 重新触发 loader（ErrorState 重试按钮的回调） */
  reload: () => void;
  /** 本地更新已加载数据（乐观更新 / 局部刷新用） */
  setData: (data: T) => void;
}

/**
 * @param loader  返回 Promise 的取数函数；deps 变化或 reload() 时重新执行
 * @param deps    触发重新加载的依赖数组（同 useEffect 语义）
 */
export function useAsyncState<T>(
  loader: () => Promise<T>,
  deps: unknown[] = [],
): AsyncState<T> {
  const [data, setData] = useState<T | null>(null);
  const [status, setStatus] = useState<AsyncStatus>("loading");
  const [error, setError] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);

  // loader 通过 ref 透传，避免调用方每渲染传新函数引用导致循环触发
  const loaderRef = useRef(loader);
  loaderRef.current = loader;

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    setError(null);
    loaderRef
      .current()
      .then((result) => {
        if (cancelled) return;
        setData(result);
        setStatus("success");
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
        setStatus("error");
      });
    return () => {
      cancelled = true;
    };
    // deps 由调用方声明取数依赖；nonce 驱动手动 reload
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, nonce]);

  const reload = useCallback(() => setNonce((n) => n + 1), []);
  const applyData = useCallback((d: T) => setData(d), []);

  return { data, status, error, reload, setData: applyData };
}
