import { useEffect, useRef, useState, type TouchEvent } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import {
  Library,
  Cpu,
  Mic,
  ScanText,
  GraduationCap,
  ArrowLeft,
  ArrowRight,
  Settings2,
  type LucideIcon,
} from "lucide-react";
import { cn } from "../../utils/cn";
import { useLayoutMode } from "../../hooks/useLayoutMode";

type StepKey = "import" | "ai" | "asr" | "ocr" | "learn";

interface StepDef {
  key: StepKey;
  Icon: LucideIcon;
  /** 可选直达路由：该步讲配置能力（AI / ASR / OCR）时，提供「去配置」一键跳转 */
  route?: string;
}

/**
 * 五个引导步骤（v3.7.1 对齐用户诉求：如何配置 AI / ASR / OCR，以及如何学习）：
 * 入库 → 配置 AI → 配置语音（ASR）→ 配置识别（OCR）→ 学习闭环。
 * AI / ASR / OCR 三步带 route，可一键跳到对应设置页实操。
 */
const STEPS: StepDef[] = [
  { key: "import", Icon: Library },
  { key: "ai", Icon: Cpu, route: "/ai-config" },
  { key: "asr", Icon: Mic, route: "/me/asr" },
  { key: "ocr", Icon: ScanText, route: "/me/ocr" },
  { key: "learn", Icon: GraduationCap },
];

interface OnboardingProps {
  onDone: () => void;
}

export function Onboarding({ onDone }: OnboardingProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const mode = useLayoutMode();
  const large = mode === "tablet-landscape";

  const [index, setIndex] = useState(0);
  const [shown, setShown] = useState(false);
  const touchX = useRef<number | null>(null);

  const last = index === STEPS.length - 1;
  const step = STEPS[index];

  const goNext = () =>
    last ? onDone() : setIndex((i) => Math.min(i + 1, STEPS.length - 1));
  const goPrev = () => setIndex((i) => Math.max(i - 1, 0));
  const skip = () => onDone();

  // 入场动画：挂载与每次切换步骤时重新触发淡入 + 上移
  useEffect(() => {
    setShown(false);
    const id = requestAnimationFrame(() => setShown(true));
    return () => cancelAnimationFrame(id);
  }, [index]);

  // 触屏左右滑动切换（主流引导页预期交互）
  const handleTouchStart = (e: TouchEvent) => {
    touchX.current = e.touches[0].clientX;
  };
  const handleTouchEnd = (e: TouchEvent) => {
    if (touchX.current == null) return;
    const dx = e.changedTouches[0].clientX - touchX.current;
    if (Math.abs(dx) > 50) (dx < 0 ? goNext : goPrev)();
    touchX.current = null;
  };

  return (
    <div
      className={cn(
        "fixed inset-0 z-[55] flex flex-col bg-paper transition-opacity duration-500",
        shown ? "opacity-100" : "opacity-0",
      )}
      onTouchStart={handleTouchStart}
      onTouchEnd={handleTouchEnd}
    >
      {/* 顶栏：跳过 */}
      <div className="flex items-center justify-between px-6 pt-6">
        <span className="text-sm font-medium text-ink-muted">
          {t("app.name")}
        </span>
        <button
          onClick={skip}
          className="rounded-full px-4 py-1.5 text-sm font-medium text-ink-muted transition active:scale-95"
        >
          {t("onboarding.skip")}
        </button>
      </div>

      {/* 内容区：图标 + 标题 + 描述 + 可选「去配置」直达 */}
      <div className="flex flex-1 items-center justify-center px-6">
        <div
          className={cn(
            "flex w-full flex-col items-center text-center",
            large ? "max-w-md" : "max-w-sm",
          )}
        >
          <div
            className={cn(
              "flex items-center justify-center rounded-3xl bg-accent/10 text-accent transition-all duration-500",
              large ? "h-32 w-32" : "h-24 w-24",
              shown ? "translate-y-0 opacity-100" : "translate-y-3 opacity-0",
            )}
          >
            <step.Icon
              className={large ? "h-14 w-14" : "h-11 w-11"}
              strokeWidth={1.8}
            />
          </div>
          <h2
            className={cn(
              "mt-8 font-bold text-ink transition-all duration-500",
              large ? "text-3xl" : "text-2xl",
              shown ? "translate-y-0 opacity-100" : "translate-y-3 opacity-0",
            )}
          >
            {t(`onboarding.steps.${step.key}.title`)}
          </h2>
          <p
            className={cn(
              "mt-3 leading-relaxed text-ink-muted transition-all duration-500",
              large ? "text-base" : "text-sm",
              shown ? "translate-y-0 opacity-100" : "translate-y-3 opacity-0",
            )}
          >
            {t(`onboarding.steps.${step.key}.desc`)}
          </p>
          {step.route && (
            <button
              onClick={() => {
                onDone();
                navigate(step.route as string);
              }}
              className={cn(
                "mt-5 flex items-center gap-1.5 rounded-full border border-accent/40 px-5 py-2 text-sm font-medium text-accent transition active:scale-95",
                shown ? "translate-y-0 opacity-100" : "translate-y-3 opacity-0",
              )}
            >
              <Settings2 className="h-4 w-4" />
              {t("onboarding.goConfig")}
            </button>
          )}
        </div>
      </div>

      {/* 底部：指示点 + 导航；pb-20 硬编码避开 Android 系统手势条（约 50px）+ 安全间距 */}
      <div className="px-6 pb-20">
        <div className="mb-6 flex items-center justify-center gap-2">
          {STEPS.map((s, i) => (
            <span
              key={s.key}
              className={cn(
                "h-2 rounded-full transition-all duration-300",
                i === index ? "w-6 bg-accent" : "w-2 bg-line",
              )}
            />
          ))}
        </div>
        <div className="flex items-center justify-between gap-4">
          {index > 0 ? (
            <button
              onClick={goPrev}
              className="flex items-center gap-1 rounded-full px-5 py-3 text-sm font-medium text-ink-muted transition active:scale-95"
            >
              <ArrowLeft className="h-4 w-4" />
              {t("onboarding.back")}
            </button>
          ) : (
            <span />
          )}
          <button
            onClick={goNext}
            className="flex items-center gap-1 rounded-full bg-accent px-7 py-3 text-sm font-medium text-accent-fg shadow-card transition active:scale-95"
          >
            {last ? t("onboarding.getStarted") : t("onboarding.next")}
            {!last && <ArrowRight className="h-4 w-4" />}
          </button>
        </div>
      </div>
    </div>
  );
}
