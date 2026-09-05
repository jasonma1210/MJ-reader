import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useLocation } from "react-router-dom";
import { useTranslation } from "react-i18next";
import {
  Send,
  BookMarked,
  Mic,
  Square,
  GraduationCap,
  Lightbulb,
  FileText,
  GitCompare,
  Play,
  Pause,
  ChevronDown,
  ChevronUp,
  X,
  Sparkles,
  Plus,
  Trash2,
  CheckCircle2,
} from "lucide-react";
import { useVoiceInput } from "../../hooks/useVoiceInput";
import { toast } from "../../utils/toast";
import { Sheet } from "../ui/Sheet";
import { useAiStore } from "../../stores/aiStore";
import { useLibraryStore } from "../../stores/libraryStore";
import { AIChatList } from "../ai/AIChatList";
import { breakdownService } from "../../services/breakdownService";
import { bookService } from "../../services/bookService";
import { playTts, stopTts, getTtsState, subscribeTts } from "../../services/ttsEngine";
import type { BreakdownChunk, Book } from "../../types";
import { cn } from "../../utils/cn";
import { routeInput, allVerbs, verbPrompt, type VerbId } from "../../ai/router";
import { logError } from "../../utils/logError";
import { useBreakpoint } from "../../hooks/useLayoutMode";

/**
 * 学习工具菜单配置：每项包含图标、标题、systemPrompt（AI 角色+阶段指令）+ initialPrompt（AI 开场白）。
 * systemPrompt 与 initialPrompt 在 useTool 中组合为一条引导消息发送，驱动 AI 分阶段引导学习者完成闭环。
 */
const LEARNING_TOOLS = [
  {
    key: "feynman",
    icon: GraduationCap,
    titleKey: "ai.tools.feynman.title",
    descKey: "ai.tools.feynman.desc",
    systemPrompt: `你是一名费曼学习法教练。你的任务是引导用户用最简单的语言讲解一个概念，暴露理解盲区，逐步深化。严格按以下阶段引导，不要跳步：

- **阶段一（选概念 + 自我认知）**：问用户想讲解哪个概念/知识点，同时了解用户当前水平（入门/熟悉/精通）。根据水平调整追问深度。
- **阶段二（让用户讲解）**：让用户用最简单的话——像对一个完全不懂的朋友讲一样——解释这个概念，不要用专业术语。
- **阶段三（追问暴露盲区）**：听完用户讲解后，提出 2-3 个追问，比如"你说的 X 和 Y 有什么区别？""举个生活中的例子""如果让反对你的人来质疑，他会说什么？"，帮助用户暴露理解盲区。
- **阶段四（类比深化）**：用一个贴切的类比/比喻帮用户打通理解，比如"你可以把 X 想象成……"。
- **阶段五（举一反三）**：给用户一个同领域相关但稍有不同的概念，让用户试试用同样方法讲解，检验迁移能力。
- **退出/完成**：当用户说"退出""结束"或明显完成了学习时，给出：① 能力评估（当前理解深度 + 盲区）；② 建议用户把这个概念存入记忆宫殿（描述与前置概念、易混概念的关系）；③ 下一步学习方向建议。提示用户"我可以把这段对话中你讲解过的概念自动提炼到记忆宫殿中，方便下次回顾"，并自动触发记忆提取。`,
    initialPrompt: "🎯 费曼学习模式已开启！\n\n在开始之前，请告诉我：\n1️⃣ 你想讲解哪个概念或知识点？（可以从上面选择书籍里的概念，也可以直接说）\n2️⃣ 你觉得自己对它了解到什么程度？（入门 / 熟悉 / 精通）",
  },
  {
    key: "analogy",
    icon: Lightbulb,
    titleKey: "ai.tools.analogy.title",
    descKey: "ai.tools.analogy.desc",
    systemPrompt: `你是一名类比思维教练。你的任务是引导用户通过生活类比来理解抽象概念，让用户自己先尝试，再用 AI 提供的类比启发思考。严格按以下阶段引导：

- **阶段一（选概念 + 找熟悉场景）**：问用户想理解哪个抽象概念/机制，以及用户熟悉的一个生活场景或日常事物。
- **阶段二（让用户自己想）**：让用户先尝试自己找一个类比，哪怕很粗糙也没关系。这步非常重要——自己想过类比的人理解更深。
- **阶段三（AI 提供 2 个类比）**：如果用户想不出或类比不够好，提供 2 个不同角度的类比（一个具象、一个偏机制），让用户判断哪个更好、为什么。
- **阶段四（让用户修改优化）**：让用户基于 AI 给的类比，修改或融合，产出一个"属于自己"的更好的类比。
- **阶段五（举一反三）**：让用户试试用这个类比框架解释另一个相关但不同的概念，检验类比的迁移能力。
- **退出/完成**：给出：① 类比能力评估（是否能独立建立有效映射）；② 建议把这个类比存入记忆宫殿（关联概念 → 类比锚点）；③ 下一步可以尝试类比的概念。提示用户"我可以把这段对话中你用到的类比自动提炼到记忆宫殿中，下次复习时能直接看到"，并自动触发记忆提取。`,
    initialPrompt: `💡 类比解释模式已开启！\n\n请告诉我：\n1️⃣ 你想理解哪个抽象概念或机制？\n2️⃣ 你熟悉的一个生活场景或日常事物是什么？（比如"快递流程""手机拍照""厨房做菜"……）`,
  },
  {
    key: "caseStudy",
    icon: FileText,
    titleKey: "ai.tools.case.title",
    descKey: "ai.tools.case.desc",
    systemPrompt: `你是一名案例分析教练。你的任务是引导用户用结构化框架拆解真实案例，从"看热闹"到"看门道"。严格按以下阶段引导：

- **阶段一（选领域 + 找案例）**：问用户想拆解哪个领域的问题/现象，以及是否有一个具体感兴趣的案例（书名、电影、商业故事、历史事件等）。如果用户没具体案例，推荐一个与用户关注领域相关的。
- **阶段二（让用户初步分析）**：先让用户谈谈"你觉得这个案例的关键是什么？最吸引你的点在哪里？"，了解用户的初始视角。
- **阶段三（选择分析框架）**：提供 3 个适合该案例的分析框架让用户选（如 SWOT / 5W2H / 因果链 / 时间线 / 利益相关者图），每个用一句话说明适用场景。如果用户没偏好，推荐一个最贴合的。
- **阶段四（用框架深入拆解）**：用选定的框架一步步拆解，每个维度都先让用户自己填，再补充深化。
- **阶段五（举一反三）**：让用户用同样框架分析一个类似但不同的案例，检验框架迁移能力。
- **退出/完成**：给出：① 案例分析能力评估（是否能独立选框架、抓关键、挖深层原因）；② 建议把案例的关键洞察存入记忆宫殿（关联知识点 → 典型案例）；③ 下一步推荐分析的案例。提示用户"我可以把这段对话中你拆解过的案例洞察自动提炼到记忆宫殿中"，并自动触发记忆提取。`,
    initialPrompt: "📖 案例拆解模式已开启！\n\n请告诉我：\n1️⃣ 你想拆解哪个领域的问题或现象？\n2️⃣ 有没有一个你感兴趣的具体案例？（比如某本书、某部电影、某个商业故事、某个历史事件）如果没有，告诉我你的学习方向，我来推荐。",
  },
  {
    key: "compare",
    icon: GitCompare,
    titleKey: "ai.tools.contrast.title",
    descKey: "ai.tools.contrast.desc",
    systemPrompt: `你是一名概念辨析教练。你的任务是引导用户清晰区分易混淆概念，从"模糊感觉不同"到"精准说出差异在哪里"。严格按以下阶段引导：

- **阶段一（选对比对象）**：问用户想对比哪两个（或更多）容易混淆的概念。如果用户不确定选什么，可以根据用户正在学习的领域推荐 1-2 组典型易混概念。
- **阶段二（让用户先说区别）**：先让用户说说"你觉得它们的区别是什么？哪怕只是感觉上的不同也行"，了解用户当前的混淆点在哪里。
- **阶段三（维度化对比）**：用表格/维度方式列出核心对比维度（定义、核心特征、适用场景、优缺点、常见误区），每个维度都让用户先验证或修正，再补充正确理解。
- **阶段四（找核心差异 + 类比固化）**：帮用户提炼出 1-2 个最核心、最本质的差异点，并用一个生活类比帮用户彻底固化这个区分（比如"A 就像单人自行车，B 就像双人自行车——核心差异是承载人数"）。
- **阶段五（举一反三）**：给出另一组相关的易混概念，让用户用同样的维度化方法尝试自己对比，检验辨析能力。
- **退出/完成**：给出：① 辨析能力评估（是否能独立找出核心差异）；② 建议把这组概念的对比关系存入记忆宫殿（标记为"易混辨析"关系）；③ 下一步推荐辨析的概念组。提示用户"我可以把这段对话中你辨析过的概念关系自动提炼到记忆宫殿中"，并自动触发记忆提取。`,
    initialPrompt: `⚖️ 对比分析模式已开启！\n\n请告诉我：\n1️⃣ 你想对比哪两个（或更多）容易混淆的概念？（比如"动机 vs 效果""监督学习 vs 强化学习"）\n2️⃣ 你是在学习哪本书或哪个主题时遇到的困惑？（方便我更有针对性地帮你）`,
  },
  {
    key: "selfCheck",
    icon: CheckCircle2,
    titleKey: "ai.tools.selfcheck.title",
    descKey: "ai.tools.selfcheck.desc",
    systemPrompt: `你是一名学习自检教练。你的任务是引导用户通过主动自测来检验自己的真实理解程度，而不是被动接受讲解。严格按以下阶段引导：

- **阶段一（选定范围）**：问用户想自测哪个概念/章节/知识点（可以是一本书的某一章、某个理论、某个公式等）。如果用户不确定，建议用户输入正在学习的书籍名称，让 AI 根据书籍内容推荐适合自测的范围。
- **阶段二（AI 出题 · 用户作答）**：根据用户选定的范围，出 3-5 道分层递进的自检题——每道题先让用户自己回答。题目类型建议：① 基础定义题（确认最基本概念）；② 应用题（能否用这个概念解释具体场景）；③ 反例题（什么情况下这个概念不适用）；④ 关联题（与前置/相邻概念的关系）。
- **阶段三（逐题反馈 + 追问）**：用户回答后，逐题给出：① 正确与否判断 + 简短原因；② 如果答错，解释为什么错并追问相关前置知识；③ 如果答对，追问一个稍微深入的延伸问题检验是否真懂。重点是**让用户在答错后马上修正认知，而不是等全部做完再给答案**。
- **阶段四（盲区定位）**：根据用户的答题表现，总结出：① 用户真正掌握得好的部分（强化自信）；② 用户存在盲区/误解的部分（需要针对性补学）；③ 哪些是"感觉懂了但说不清楚"的灰区。
- **阶段五（针对性练习）**：针对定位出的盲区，推荐 1-2 个聚焦练习——可以是：用费曼学习法重新讲这个盲区概念、找一个类比来理解它、或者看这个盲区相关的推荐阅读片段。
- **退出/完成**：给出完整自检报告：① 本次自检覆盖了哪些知识点；② 答对/答错数量 + 关键盲区；③ 具体的下一步行动清单（建议存入记忆宫殿的知识点 + 推荐继续学习的方向 + 是否建议切换到其他学习工具）；④ 鼓励用户把这次自检的错题存入错题集，后续可以针对性复习。⑤ 提示用户"我可以把这段对话中你答错的知识点和盲区自动提炼到记忆宫殿中，方便下次针对性复习"，并自动触发记忆提取。

重要原则：
- 永远先让用户自己回答，再给反馈。不要直接给答案。
- 答错后先共情（"这个问题确实容易混淆""很多人都会在这里卡住"），再给纠正。
- 关注用户的**认知过程**而不只是最终答案——用户是怎么想的、为什么会这么想，比对错更重要。`,
    initialPrompt: "🧠 自查理解模式已开启！\n\n这个模式通过主动自测帮你发现自己真正掌握了什么、还卡在什么地方——比被动看书更能暴露盲区。\n\n请告诉我：\n1️⃣ 你想自测哪个概念、哪一章或哪个知识点？（建议选具体的，比如\"第三章的需求分析\"而不是\"整本书\"）\n2️⃣ 这个内容来自哪本书或哪个主题？",
  },
];

/** V2 动词快捷触发清单：「问」是默认对话行为不单列，其余 7 动词按钮直触发 */
const QUICK_VERBS = allVerbs().filter((v) => v.id !== "ask");

/**
 * 统一 AI 面板（4-Tab 共用）：聊天式交互，流式接收 token。
 * v2: 书籍选择醒目化 + 微信式语音输入 + TTS 自动播放 + 学习工具引导式 prompt。
 * 护眼跟随：Sheet 内部使用 --overlay-* token。
 */
export function AIPanel() {
  const { t } = useTranslation();
  const open = useAiStore((s) => s.open);
  const streaming = useAiStore((s) => s.streaming);
  const send = useAiStore((s) => s.send);
  const closePanel = useAiStore((s) => s.closePanel);
  const startNewConversation = useAiStore((s) => s.startNewConversation);
  const scope = useAiStore((s) => s.scope);
  const mode = useAiStore((s) => s.mode);
  const setChapter = useAiStore((s) => s.setChapter);
  const messages = useAiStore((s) => s.messages);

  const [input, setInput] = useState("");
  const [chapters, setChapters] = useState<BreakdownChunk[]>([]);
  const [bookInfo, setBookInfo] = useState<Book | null>(null);
  // 微信式输入：textMode=false → 语音输入模式
  const [textMode, setTextMode] = useState(true);
  // TTS 自动播放开关（默认开启）
  const [ttsAutoPlay, setTtsAutoPlay] = useState(true);
  const [ttsPlaying, setTtsPlaying] = useState(false);
  // 学习工具菜单展开状态
  const [toolsOpen, setToolsOpen] = useState(false);
  // 学习工具按钮 ref（用于 Portal 定位）
  const toolsBtnRef = useRef<HTMLButtonElement>(null);
  // Portal 弹出菜单位置
  const [toolsMenuPos, setToolsMenuPos] = useState({ top: 0, left: 0, width: 0 });
  // 书籍选择弹窗
  const [bookPickerOpen, setBookPickerOpen] = useState(false);
  // 书籍列表（用于书籍选择器）
  const books = useLibraryStore((s) => s.books);
  const autoSentRef = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  // 按住说话：是否处于"按住中"（useState 用于渲染 + useRef 用于异步时序安全判断）
  const [pressing, setPressing] = useState(false);
  const pressingRef = useRef(false);
  // 按住说话：是否已上滑到取消阈值
  const [cancelling, setCancelling] = useState(false);
  const cancellingRef = useRef(false);
  const pressStartYRef = useRef<number>(0);
  const pressTimerRef = useRef<number>(0);

  // ===== 预填（错题追问等一键提问） =====
  useEffect(() => {
    if (scope.prefill) setInput(scope.prefill);
  }, [scope.prefill]);

  // ===== 拉取书籍信息 =====
  useEffect(() => {
    if (open && scope.bookId) {
      void Promise.all([
        breakdownService.getResult(scope.bookId),
        bookService.getBookById(scope.bookId),
      ]).then(([r, book]) => {
        setChapters(r?.chunks ?? []);
        setBookInfo(book);
      });
    } else {
      setBookInfo(null);
    }
  }, [open, scope.bookId]);

  // ===== 选区单轮（总结/翻译/解释）+ V2 动词发起位（autoSend，chat 承载） =====
  useEffect(() => {
    if (!open) {
      autoSentRef.current = false;
      return;
    }
    const singleShot = mode === "summary" || mode === "translate" || mode === "explain";
    const shouldAutoSend = singleShot || scope.autoSend === true;
    if (!shouldAutoSend || !scope.selectionText || autoSentRef.current) return;
    autoSentRef.current = true;
    // 自动发起（选区总结/翻译/解释）同样先停旧播报
    stopTts({ owner: "ai" });
    setTtsPlaying(false);
    void send(scope.selectionText);
  }, [open, mode, scope.selectionText, scope.autoSend, send]);

  // ===== TTS 自动播放：streaming 结束后自动朗读最后一条 AI 消息 =====
  // v3.7.2 修复「退出 AI 界面仍在播报」：AIPanel 是 Shell 层常驻浮层（永不卸载），
  // 此前播放完全不受面板显隐约束——关闭面板、切走 tab 后 Edge TTS 仍在读。
  // 现在朗读生命周期严格绑定：面板可见(open) 且在 AI 路由内；任一条件不满足立即停止。
  const lastAiTextRef = useRef<string>("");
  /** 已自动朗读过的文本：同一条回复只自动读一次，避免切开关/重挂载后重复播报。 */
  const autoSpokenRef = useRef<string>("");
  useEffect(() => {
    // 收集最后一条 AI 消息文本
    const lastMsg = [...messages].reverse().find((m) => m.role === "assistant");
    if (lastMsg && "content" in lastMsg) {
      lastAiTextRef.current = (lastMsg.content as string) || "";
    }
  }, [messages]);

  // 新一轮流式开始时清空去重标记（新回复允许再次自动朗读）
  useEffect(() => {
    if (streaming) autoSpokenRef.current = "";
  }, [streaming]);

  useEffect(() => {
    if (!open || !ttsAutoPlay || streaming) return;
    const text = lastAiTextRef.current.trim();
    if (text.length <= 20) return;
    if (autoSpokenRef.current === text) return;
    // 延迟 500ms 让 UI 稳定；期间面板可能已被关闭，起播前再次确认。
    const timer = setTimeout(() => {
      if (!useAiStore.getState().open) return;
      autoSpokenRef.current = text;
      playTts(text, { owner: "ai" });
    }, 500);
    return () => clearTimeout(timer);
  }, [open, streaming, ttsAutoPlay, messages]);

  // 面板关闭（点遮罩/关闭按钮/返回键）→ 立即停止 AI 播报（只停 AI，不动阅读器朗读）
  useEffect(() => {
    if (!open) {
      stopTts({ owner: "ai" });
      setTtsPlaying(false);
    }
  }, [open]);

  // 离开 AI 路由（切底部 tab / 导航到别的页面）→ 立即停止 AI 播报。
  // 浮层常驻可能仍可见，但已不属于「AI 界面」，按用户要求必须静音。
  const { pathname } = useLocation();
  useEffect(() => {
    if (!pathname.startsWith("/ai")) {
      stopTts({ owner: "ai" });
      setTtsPlaying(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pathname]);

  // 组件卸载兜底：任何原因卸载都不允许留下正在播报的 AI 音频
  useEffect(() => () => stopTts({ owner: "ai" }), []);

  // 订阅 TTS 状态更新 UI
  useEffect(() => {
    const unsub = subscribeTts(() => {
      const s = getTtsState();
      // 仅当朗读归属为 AI 时才把面板按钮显示为「播放中」，避免阅读器朗读串味到 AI 开关
      setTtsPlaying(s.isPlaying && s.owner === "ai");
    });
    return unsub;
  }, []);

  // ===== 自动滚动到底部 =====
  useEffect(() => {
    const el = scrollRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }, [messages]);

  // ===== 语音输入（WebRTC → 后端 transcribe） =====
  // 识别成功后：清空输入框 → 自动发送对话（按住说话 → 松开 → 识别 → 发送，与微信语音条体验一致）
  const voice = useVoiceInput((text) => {
    setInput("");
    // 新一轮提问前停止旧回复播报（v3.7.2）：否则新回答与旧音频叠加
    stopTts({ owner: "ai" });
    setTtsPlaying(false);
    void send(text);
  });

  // ===== 发送 =====
  const submit = () => {
    const text = input;
    setInput("");
    setToolsOpen(false);
    // 发新提问时停止上一条回复的播报
    stopTts({ owner: "ai" });
    setTtsPlaying(false);
    // V2 动词路由：命中学习动词（考我/拆书/制卡…）时转换为教练式 prompt，否则原样对话
    const bookTitle = scope.scope === "book" ? (bookInfo?.title ?? null) : null;
    void send(routeInput(text, bookTitle));
  };

  // ===== 学习工具引导式 prompt =====
  const handleUseTool = (tool: typeof LEARNING_TOOLS[number]) => {
    startNewConversation();
    setToolsOpen(false);
    // 开启学习工具即切换对话语境，停止旧播报
    stopTts({ owner: "ai" });
    setTtsPlaying(false);
    const combined = `${tool.systemPrompt}\n\n---\n\n现在请以教练身份开始引导。开场白：\n${tool.initialPrompt}`;
    void send(combined);
  };

  // ===== V2 动词快捷触发：按钮直触发对应动词 prompt =====
  const handleQuickVerb = (id: VerbId) => {
    startNewConversation();
    // 同上：动词触发新一轮，停止旧播报
    stopTts({ owner: "ai" });
    setTtsPlaying(false);
    const bookTitle = scope.scope === "book" ? (bookInfo?.title ?? null) : null;
    void send(verbPrompt(id, bookTitle));
  };

  // ===== 学习工具 Portal 定位 =====
  useEffect(() => {
    if (!toolsOpen) return;
    const btn = toolsBtnRef.current;
    if (!btn) return;
    const rect = btn.getBoundingClientRect();
    // 让菜单从按钮上方弹出（bottom-full 等效）
    setToolsMenuPos({
      top: rect.top - 8,
      left: rect.left,
      width: rect.width,
    });
  }, [toolsOpen]);

  // ===== 点击外部关闭学习工具菜单 =====
  useEffect(() => {
    if (!toolsOpen) return;
    const onDocClick = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (!target.closest("[data-tools-menu]") && !target.closest("[data-tools-btn]")) {
        setToolsOpen(false);
      }
    };
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [toolsOpen]);

  // ===== 按住说话（微信式） =====
  // 注意：当前实现走 WebRTC → 后端 transcribe_audio
  // iOS 上 WebRTC 能工作（getUserMedia 在 Tauri iOS 可用）
  const handlePressStart = async (clientY: number) => {
    if (import.meta.env.DEV) console.debug(`[UI-1] handlePressStart 入口`);
    // ref 优先同步，确保异步时序安全
    pressingRef.current = true;
    cancellingRef.current = false;
    setPressing(true);
    setCancelling(false);
    pressStartYRef.current = clientY;
    pressTimerRef.current = Date.now();
    // 开始录音先停播报：否则 TTS 声音会被麦克风拾取，污染识别结果
    stopTts({ owner: "ai" });
    setTtsPlaying(false);
    const err = await voice.start();
    if (err) {
      if (import.meta.env.DEV) console.debug(`[UI-2] voice.start() 返回错误：${err}`);
      pressingRef.current = false;
      setPressing(false);
    } else {
      if (import.meta.env.DEV) console.debug(`[UI-2] voice.start() 成功`);
    }
  };

  const handlePressMove = (clientY: number) => {
    if (!pressingRef.current) return;
    const deltaY = clientY - pressStartYRef.current;
    // 上滑 > 60px 进入取消态
    const cancelNow = deltaY < -60;
    cancellingRef.current = cancelNow;
    setCancelling(cancelNow);
  };

  const handlePressEnd = async () => {
    if (import.meta.env.DEV) console.debug(`[UI-3] handlePressEnd 入口（pressingRef=${pressingRef.current}, cancellingRef=${cancellingRef.current}）`);
    if (!pressingRef.current) {
      if (import.meta.env.DEV) console.debug(`[UI-3b] 守卫未通过：pressingRef=false，跳过 stop`);
      return;
    }
    const wasCancelling = cancellingRef.current; // 先记住，再清理
    pressingRef.current = false;
    cancellingRef.current = false;
    setPressing(false);
    setCancelling(false);
    if (wasCancelling) {
      // 上滑取消
      try {
        voice.stop();
      } catch (e) {
        logError("AIPanel.voiceStop", e);
      }
      toast(t("ai.voice.cancelled"));
      return;
    }
    if (import.meta.env.DEV) console.debug(`[UI-4] 调用 voice.stop()`);
    const err = await voice.stop();
    if (import.meta.env.DEV) console.debug(`[UI-5] voice.stop() ${err ? `错误：${err}` : "成功"}`);
  };

  // ===== 渲染 =====
  // 中枢三形态（V2）：桌读横屏=右侧抽屉；随身/平板竖屏=底部 Sheet；工作台由 ReaderAiPanel 常驻右栏
  const layout = useBreakpoint();
  return (
    <>
    <Sheet open={open} onClose={closePanel} title={t("ai.title")} variant={layout.isTabletLandscape ? "right" : "bottom"}>
      <div className={cn("flex flex-col gap-3", layout.isTabletLandscape ? "h-full" : "h-[70vh]")}>
        {/* ==== 顶部：TTS ==== */}
        <div className="flex justify-end items-center gap-2 border-b border-overlay pb-2">
          {/* TTS 开关 */}
          <button
            type="button"
            onClick={() => {
              if (ttsPlaying) {
                // 只停 AI 自己的播报（阅读器正文朗读不受影响）
                stopTts({ owner: "ai" });
                setTtsPlaying(false);
              } else {
                setTtsAutoPlay((v) => !v);
              }
            }}
            title={ttsPlaying ? t("ai.tts.stop") : ttsAutoPlay ? t("ai.tts.onState") : t("ai.tts.offState")}
            className={cn(
              "flex items-center gap-1 rounded-full border px-2.5 py-1 text-[11px] transition",
              ttsPlaying
                ? "border-danger bg-danger/10 text-danger"
                : ttsAutoPlay
                  ? "border-accent/50 bg-accent/10 text-accent"
                  : "border-overlay bg-overlay text-ink-muted hover:bg-line-soft",
            )}
          >
            {ttsPlaying ? (
              <Pause className="h-3 w-3" />
            ) : (
              <Play className="h-3 w-3" />
            )}
            {ttsPlaying ? t("ai.tts.playing") : ttsAutoPlay ? t("ai.tts.on") : t("ai.tts.off")}
          </button>
        </div>

        {/* ==== 消息列表 ==== */}
        <div ref={scrollRef} className="flex-1 space-y-3 overflow-auto">
          <AIChatList />
        </div>

        {/* ==== 章节选择器 ==== */}
        {scope.bookId && chapters.length > 0 && (
          <div className="flex items-center gap-1.5 overflow-x-auto border-t border-overlay pt-2">
            <BookMarked className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
            <button
              onClick={() => setChapter(null)}
              className={cn(
                "shrink-0 rounded-full px-2 py-1 text-[10px] font-medium transition",
                scope.chapterIndex === null || scope.chapterIndex === undefined
                  ? "bg-accent text-accent-fg"
                  : "bg-overlay text-ink-muted",
              )}
            >
              {t("ai.panel.allBook")}
            </button>
            {chapters.map((c) => (
              <button
                key={c.chapterIndex}
                onClick={() => setChapter(c.chapterIndex)}
                className={cn(
                  "shrink-0 rounded-full px-2 py-1 text-[10px] font-medium transition",
                  scope.chapterIndex === c.chapterIndex
                    ? "bg-accent text-accent-fg"
                    : "bg-overlay text-ink-muted",
                )}
              >
                {c.chapterTitle.length > 8
                  ? c.chapterTitle.slice(0, 8) + "…"
                  : c.chapterTitle}
              </button>
            ))}
          </div>
        )}

        {/* ==== 选择书籍 + 学习工具（输入框上方）==== */}
        <div className="flex flex-col gap-2">
          {/* 选择书籍 - 第一行 */}
          <button
            type="button"
            onClick={() => setBookPickerOpen(true)}
            className={cn(
              "flex w-full items-center gap-2 rounded-[var(--radius-md)] border px-3 py-2 text-sm font-medium transition",
              scope.bookId && bookInfo
                ? "border-accent bg-accent/10 text-accent"
                : "border-overlay bg-overlay text-ink-muted hover:border-accent/50",
            )}
          >
            <BookMarked className="h-4 w-4 shrink-0" />
            {scope.bookId && bookInfo ? (
              <span className="flex-1 truncate text-left">{bookInfo.title}</span>
            ) : (
              <span className="flex-1 text-left">{t("ai.bookPicker.entry")}</span>
            )}
            {scope.bookId && bookInfo && (
              <X
                className="h-3.5 w-3.5 shrink-0 cursor-pointer"
                onClick={(e) => {
                  e.stopPropagation();
                  // v3.7.2 修复：解除绑定必须走 applyBookScope(undefined) 清掉
                  // scope.bookId——此前只清本地 bookInfo/chapters，请求仍带旧书，
                  // 造成「UI 显示未绑定、实际仍绑定」的状态错位。
                  useAiStore.getState().applyBookScope(undefined);
                }}
              />
            )}
          </button>

          {/* 学习工具 - 第二行 */}
          <div className="relative self-start">
            <button
              ref={toolsBtnRef}
              data-tools-btn
              type="button"
              onClick={() => setToolsOpen((v) => !v)}
              className="flex h-10 items-center gap-1 rounded-[var(--radius-md)] border border-overlay bg-overlay px-3 text-xs text-ink-muted transition hover:bg-line-soft"
            >
              <Sparkles className="h-4 w-4" />
              学习工具
              {toolsOpen ? (
                <ChevronUp className="h-3 w-3" />
              ) : (
                <ChevronDown className="h-3 w-3" />
              )}
            </button>
          </div>
        </div>

        {/* ==== 输入区 ==== */}
        <div className="flex items-center gap-2 border-t border-overlay pt-3">
          {/* 文本/语音模式切换 */}
          <button
            type="button"
            onClick={() => {
              // 切换模式时停止 AI 播报（不误伤阅读器朗读）
              stopTts({ owner: "ai" });
              setTextMode((v) => !v);
            }}
            title={textMode ? t("ai.input.toVoice") : t("ai.input.toText")}
            className={cn(
              "flex h-10 w-10 shrink-0 items-center justify-center rounded-[var(--radius-md)] transition",
              textMode
                ? "bg-overlay text-ink-soft hover:bg-line-soft"
                : "bg-accent text-accent-fg",
            )}
          >
            <Mic className="h-5 w-5" />
          </button>

          {textMode ? (
            /* 文本输入模式 */
            <>
              <input
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    submit();
                  }
                }}
                placeholder={t("ai.placeholder")}
                className="flex-1 rounded-[var(--radius-md)] border border-overlay bg-overlay px-3 py-2 text-sm text-overlay outline-none focus:border-accent"
              />
              <button
                type="button"
                onClick={submit}
                disabled={streaming || !input.trim()}
                aria-label={t("ai.send")}
                className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[var(--radius-md)] bg-accent text-accent-fg disabled:opacity-50"
              >
                <Send className="h-5 w-5" />
              </button>
            </>
          ) : (
            /* 语音输入模式：按住说话 */
            <>
              <button
                type="button"
                onMouseDown={(e) => handlePressStart(e.clientY)}
                onMouseMove={(e) => handlePressMove(e.clientY)}
                onMouseUp={handlePressEnd}
                onMouseLeave={handlePressEnd}
                onTouchStart={(e) => handlePressStart(e.touches[0].clientY)}
                onTouchMove={(e) => handlePressMove(e.touches[0].clientY)}
                onTouchEnd={handlePressEnd}
                onTouchCancel={handlePressEnd}
                disabled={streaming || voice.busy}
                className={cn(
                  "flex-1 select-none rounded-[var(--radius-md)] border text-center py-2 text-sm font-medium transition disabled:opacity-50",
                  cancelling
                    ? "border-danger bg-danger/10 text-danger"
                    : pressing
                      ? "border-accent bg-accent text-accent-fg"
                      : "border-overlay bg-overlay text-ink-soft",
                )}
              >
                {cancelling
                  ? t("ai.input.releaseToCancel")
                  : pressing
                    ? t("ai.input.recording")
                    : voice.recording
                      ? t("ai.input.stopRecording")
                      : t("ai.input.holdToTalk")}
              </button>
              {/* 紧急停止按钮（录音中显示） */}
              {voice.recording && !pressing && (
                <button
                  type="button"
                  onClick={handlePressEnd}
                  className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[var(--radius-md)] bg-danger text-white"
                >
                  <Square className="h-5 w-5" />
                </button>
              )}
            </>
          )}
        </div>
      </div>

      {/* ==== 书籍选择弹窗 ==== */}
      {bookPickerOpen && (
        <div
          className="fixed inset-0 z-[100] flex items-end justify-center bg-black/40"
          onClick={() => setBookPickerOpen(false)}
        >
          <div
            className="w-full max-w-md rounded-t-2xl border-t border-overlay bg-paper p-4 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="mb-3 flex items-center justify-between">
              <span className="text-sm font-semibold text-ink">{t("ai.bookPicker.entry")}</span>
              <button
                type="button"
                onClick={() => setBookPickerOpen(false)}
                className="rounded-full p-1 text-ink-muted hover:bg-line-soft"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            {/* 当前书籍快捷操作 */}
            {scope.bookId && (
              <button
                type="button"
                onClick={() => {
                  useAiStore.getState().applyBookScope(undefined);
                  setBookPickerOpen(false);
                }}
                className="mb-3 flex w-full items-center gap-2 rounded-lg border border-accent/30 bg-accent/5 p-3 text-left"
              >
                <span className="flex-1 text-sm font-medium text-ink">{t("ai.bookPicker.global")}</span>
                <X className="h-4 w-4 text-danger" />
              </button>
            )}

            {/* 书籍列表 */}
            <div className="max-h-[50vh] overflow-y-auto">
              {books.length === 0 ? (
                <div className="py-6 text-center text-xs text-ink-muted">
                  书库还没有书籍，先去书架添加一本书吧
                </div>
              ) : (
                <div className="flex flex-col gap-1">
                  {books.map((book) => {
                    const active = scope.bookId === book.id;
                    return (
                      <button
                        key={book.id}
                        type="button"
                        onClick={() => {
                          useAiStore.getState().applyBookScope(book.id);
                          setBookPickerOpen(false);
                        }}
                        className={cn(
                          "flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left transition",
                          active
                            ? "bg-accent/10 text-accent"
                            : "hover:bg-paper-soft text-ink",
                        )}
                      >
                        <BookMarked className="h-4 w-4 shrink-0" />
                        <div className="min-w-0 flex-1">
                          <div className="truncate text-sm font-medium">{book.title}</div>
                          {book.author && (
                            <div className="truncate text-[11px] text-ink-muted">{book.author}</div>
                          )}
                        </div>
                        {active && <div className="h-2 w-2 shrink-0 rounded-full bg-accent" />}
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          </div>
        </div>
      )}
    </Sheet>
    {/* 学习工具下拉菜单 —— Portal 渲染到 body，避开 Sheet overflow-hidden 裁剪 */}
    {toolsOpen &&
      typeof document !== "undefined" &&
      createPortal(
        <div
          data-tools-menu
          style={{
            position: "fixed",
            top: Math.max(toolsMenuPos.top - 220, 8),
            left: Math.min(toolsMenuPos.left, window.innerWidth - 264),
            width: 264,
            zIndex: 9999,
          }}
          className="rounded-lg border border-line bg-paper p-1 shadow-xl"
        >
          {/* V2 动词快捷触发：学习动词按钮直触发（问为默认对话行为，不单列） */}
          <div className="mb-1 border-b border-line pb-1">
            <div className="px-2 pb-1 text-[10px] text-ink-muted">
              {t("ai.verbs.section")}
            </div>
            <div className="flex flex-wrap gap-1 px-1 pb-1">
              {QUICK_VERBS.map((v) => (
                <button
                  key={v.id}
                  type="button"
                  onClick={() => {
                    setToolsOpen(false);
                    handleQuickVerb(v.id);
                  }}
                  className="rounded-full border border-line px-2 py-1 text-[11px] text-ink-soft transition hover:bg-paper-soft"
                >
                  {t(v.labelKey)}
                </button>
              ))}
            </div>
          </div>
          {LEARNING_TOOLS.map((tool) => {
            const Icon = tool.icon;
            return (
              <button
                key={tool.key}
                type="button"
                onClick={() => {
                  setToolsOpen(false);
                  handleUseTool(tool);
                }}
                className="flex w-full items-start gap-2 rounded-md p-2 text-left transition hover:bg-paper-soft"
              >
                <Icon className="mt-0.5 h-4 w-4 shrink-0 text-accent" />
                <div className="min-w-0 flex-1">
                  <div className="text-xs font-medium text-ink">{t(tool.titleKey)}</div>
                  <div className="text-[10px] text-ink-muted truncate">{t(tool.descKey)}</div>
                </div>
              </button>
            );
          })}
        </div>,
        document.body,
      )}
    </>
  );
}
