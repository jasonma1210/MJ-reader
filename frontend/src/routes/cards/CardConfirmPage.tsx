import { useNavigate, useSearchParams } from "react-router-dom";
import { FlashcardConfirmSheet } from "../../components/learn/FlashcardConfirmSheet";
import { parseCardConfirmParams } from "../../components/learn/flashcardConfirmLogic";

/**
 * /cards/new 路由壳：只负责解析入口参数与返回导航，
 * 真正的核验交互在 FlashcardConfirmSheet（components/learn）中。
 */
export function CardConfirmPage() {
  const [search] = useSearchParams();
  const navigate = useNavigate();
  const params = parseCardConfirmParams(search.toString());

  const close = () => {
    if (window.history.length > 1) navigate(-1);
    else navigate("/");
  };

  return (
    <div className="h-full min-h-screen bg-paper-soft">
      <FlashcardConfirmSheet
        open
        onClose={close}
        bookId={params.bookId}
        highlightIds={params.highlightIds}
        source={params.source}
      />
    </div>
  );
}
