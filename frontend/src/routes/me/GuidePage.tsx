import { useNavigate } from "react-router-dom";
import { Onboarding } from "../../components/onboarding/Onboarding";

/**
 * 使用引导页（v3.7.1）：
 * 复用首启 Onboarding 五步引导（入库 → 配置 AI → 配置 ASR → 配置 OCR → 如何学习），
 * 供「我的 → 使用引导」随时重看；完成/跳过后返回上一页。
 */
export function GuidePage() {
  const navigate = useNavigate();
  return <Onboarding onDone={() => navigate(-1)} />;
}
