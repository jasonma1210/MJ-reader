import type {
  Book,
  NoteItem,
  LearnStats,
  ReadingHeatmapCell,
  MemoryCurvePoint,
  WeakKnowledgeNode,
  AIProfile,
} from "../types";

// 浏览器 / 非 Tauri 环境下的占位数据，保证 UI 不白屏、布局/导航可达。
// 真实 Tauri 运行时由对应 service 调用后端命令覆盖。

const DAY = 24 * 60 * 60 * 1000;

export const MOCK_BOOKS: Book[] = [
  {
    id: "bk-1",
    title: "认知觉醒",
    author: "周岭",
    coverPath: null,
    filePath: "/books/cognitive-awakening.epub",
    format: "epub",
    fileSize: 2_400_000,
    tags: "自我提升,思维",
    description: "开启自我改变的原动力。",
    publisher: null,
    language: "zh",
    createdAt: Date.now() - 30 * DAY,
    updatedAt: Date.now() - 2 * DAY,
    lastReadAt: Date.now() - 2 * DAY,
    directoryId: null,
    progressPercentage: 62,
    currentChapter: "第五章 觉醒的方法是觉醒",
  },
  {
    id: "bk-2",
    title: "深入理解计算机系统",
    author: "Randal E. Bryant",
    coverPath: null,
    filePath: "/books/csapp.pdf",
    format: "pdf",
    fileSize: 18_000_000,
    tags: "计算机,经典",
    description: "从程序员视角透视计算机系统。",
    publisher: "机械工业出版社",
    language: "zh",
    createdAt: Date.now() - 60 * DAY,
    updatedAt: Date.now() - 9 * DAY,
    lastReadAt: Date.now() - 9 * DAY,
    directoryId: null,
    progressPercentage: 18,
    currentChapter: "Chapter 3 Machine-Level Representation",
  },
  {
    id: "bk-3",
    title: "The Pragmatic Programmer",
    author: "Andrew Hunt",
    coverPath: null,
    filePath: "/books/pragmatic.epub",
    format: "epub",
    fileSize: 3_100_000,
    tags: "编程,工程",
    description: "Your journey to mastery.",
    publisher: "Addison-Wesley",
    language: "en",
    createdAt: Date.now() - 12 * DAY,
    updatedAt: Date.now() - 20 * DAY,
    lastReadAt: null,
    directoryId: null,
    progressPercentage: 0,
    currentChapter: null,
  },
  {
    id: "bk-4",
    title: "人类简史",
    author: "尤瓦尔·赫拉利",
    coverPath: null,
    filePath: "/books/sapiens.txt",
    format: "txt",
    fileSize: 1_200_000,
    tags: "历史,社科",
    description: "从动物到上帝。",
    publisher: null,
    language: "zh",
    createdAt: Date.now() - 90 * DAY,
    updatedAt: Date.now() - 40 * DAY,
    lastReadAt: Date.now() - 40 * DAY,
    directoryId: null,
    progressPercentage: 100,
    currentChapter: "尾声 智人的末日",
  },
];

export const MOCK_NOTES: NoteItem[] = [
  {
    id: "nt-1",
    bookId: "bk-1",
    bookTitle: "认知觉醒",
    kind: "highlight",
    excerpt: "觉醒的本质是主动跳出舒适区。",
    content: "我想用它来督促自己每天复盘。",
    tags: ["方法", "习惯"],
    createdAt: Date.now() - 2 * DAY,
  },
  {
    id: "nt-2",
    bookId: "bk-2",
    bookTitle: "深入理解计算机系统",
    kind: "summary",
    excerpt: "Chapter 3 讲机器级表示。",
    content: "数据在内存中以小端序存放，调试时要留意。",
    tags: ["CSAPP"],
    createdAt: Date.now() - 9 * DAY,
  },
  {
    id: "nt-3",
    bookId: "bk-2",
    bookTitle: "深入理解计算机系统",
    kind: "wrong",
    excerpt: "浮点运算顺序不满足结合律。",
    content: "练习题踩坑：3.14 + 1e16 - 1e16 ≠ 3.14。",
    tags: ["浮点"],
    createdAt: Date.now() - 8 * DAY,
  },
];

export const MOCK_STATS: LearnStats = {
  totalSeconds: 720 * 60,
  totalPages: 340,
  booksRead: 1,
  todaySeconds: 60 * 60,
  weekSeconds: 5 * 3600,
  monthSeconds: 20 * 3600,
  dueCards: 8,
  streakDays: 18,
};

export const MOCK_HEATMAP: ReadingHeatmapCell[] = Array.from(
  { length: 84 },
  (_, i) => {
    const date = new Date(Date.now() - (83 - i) * DAY);
    return {
      date: date.toISOString().slice(0, 10),
      count: Math.round(Math.abs(Math.sin(i / 4)) * 5),
    };
  },
);

export const MOCK_CURVE: MemoryCurvePoint[] = Array.from(
  { length: 12 },
  (_, i) => ({
    label: `W${i + 1}`,
    value: Math.round(40 + Math.sin(i / 2) * 25 + i * 2),
  }),
);

export const MOCK_WEAK: WeakKnowledgeNode[] = [
  {
    id: "wk-1",
    topic: "浮点数表示",
    mastery: 0.35,
    bookId: "bk-2",
    linkedCardIds: ["nt-3"],
  },
  {
    id: "wk-2",
    topic: "链接与重定位",
    mastery: 0.2,
    bookId: "bk-2",
    linkedCardIds: [],
  },
];

export const MOCK_PROFILES: AIProfile[] = [
  {
    id: "pf-2",
    name: "OpenAI 兼容",
    provider: "openai",
    model: "gpt-4o-mini",
    enabled: true,
  },
];

export const MOCK_CHAT_REPLY =
  "这是一段来自 MJNexus Reader 的示例回复。在 Tauri 运行时中，它会替换为真实的 AI 流式输出。";

export const MOCK_REVIEW_SNAPSHOT = {
  errorQuestions: [
    {
      question: "浮点运算顺序不满足结合律，为什么？",
      knowledgePoint: "浮点数表示",
      chapter: "Chapter 3",
    },
  ],
  annotations: [
    { selectedText: "觉醒的本质是主动跳出舒适区。", note: "每天复盘", tags: ["方法"] },
    { selectedText: "数据在内存中以小端序存放。", note: "调试留意", tags: ["CSAPP"] },
  ],
  chatHistory: ["这句话怎么理解？", "帮我总结这一章", "浮点数为什么不精确？"],
};
