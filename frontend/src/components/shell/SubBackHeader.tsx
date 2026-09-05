import { useTranslation } from "react-i18next";
import { ArrowLeft } from "lucide-react";

interface SubBackHeaderProps {
  titleKey: string;
  onBack: () => void;
}

/** 二级及以下页面顶部返回栏：左上角返回箭头 + 标题。 */
export function SubBackHeader({ titleKey, onBack }: SubBackHeaderProps) {
  const { t } = useTranslation();
  return (
    <div
      className="flex shrink-0 items-center gap-2 border-b border-line bg-paper px-2 py-2"
    >
      <button
        onClick={onBack}
        aria-label={t("nav.back")}
        className="rounded-full p-2 text-ink-soft transition active:scale-95"
      >
        <ArrowLeft className="h-5 w-5" />
      </button>
      <span className="truncate font-semibold text-ink">{t(titleKey)}</span>
    </div>
  );
}
