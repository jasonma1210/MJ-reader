// 第二梯队（P1 学习深度，共用语音组件）全部命令的 service 封装。
// 对齐后端：src-tauri/src/commands/practice.rs / teaching.rs / voice_coach.rs
// （serde rename_all = camelCase）。命令名注册表见 tauri.ts（CMD.*）。
//
// 返回字段严格按后端结构对齐；浏览器预览（非 Tauri）一律返回空态，
// 生产 Tauri 构建后端异常向下透传（fail-closed，不静默 mock）。

import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

// ============ 场景化练习（F-4-002）=================
export interface PracticeSession {
  id: string;
  practiceType: string;
  targetNodeId: string | null;
  targetNodeName: string | null;
  materialBookId: string | null;
  status: string;
}

export interface PracticeEval {
  id: string;
  sessionId: string;
  practiceType: string;
  userOutput: string;
  aiFeedback: string;
  score: number;
  createdAt: number;
}

// ============ 语音问答（F-4-003）=================
export interface VoiceAsk {
  /** 会话 id：无则视为浏览器预览/未返回，无法继续作答。 */
  sessionId: string;
  question: string;
  audioUrl?: string | null;
}

export interface VoiceAnswer {
  transcribedText: string;
  aiFeedback: string;
  score: number;
  aiAudioUrl?: string | null;
}

// ============ 教学相长（F-5-002）=================
export interface TeachingMsg {
  role: string; // "user" | "assistant"
  content: string;
}

export interface TeachingSession {
  id: string;
  targetKnowledgeId: string | null;
  targetKnowledgeName: string | null;
  dialogue: TeachingMsg[];
  clarityScore: number;
  completenessScore: number;
  accuracyScore: number;
  status: string; // "active" | "done"
}

// ============ 语音 AI 教练（F-8-002）=================
export interface VoiceMsg {
  role: string; // "user" | "assistant"
  content: string;
  ts?: number | null;
}

export interface VoiceCoachSession {
  id: string;
  asrModel: string;
  ttsVoiceId: string;
  llmSystemPrompt: string;
  maxHistoryTurns: number;
  messages: VoiceMsg[];
  createdAt: number;
}

export interface VoiceCoachReply {
  sessionId: string;
  replyText: string;
  replyAudioUrl?: string | null;
}

export interface VoiceCoachInterruptResult {
  cancelled: boolean;
}

/** 兜底空返回（浏览器预览用），保持调用方无需判 Tauri */
const EMPTY_COACH_SESSION: VoiceCoachSession = {
  id: "",
  asrModel: "",
  ttsVoiceId: "",
  llmSystemPrompt: "",
  maxHistoryTurns: 0,
  messages: [],
  createdAt: 0,
};

// ============ 场景化练习 ============

/** 开始一次场景化练习会话（费曼/案例拆解/项目式/对比练习）。 */
export async function practiceScenarioStart(
  practiceType: string,
  targetNodeId?: string | null,
  materialBookId?: string | null,
): Promise<PracticeSession | null> {
  if (!isTauri()) {
    logError("practiceService.start.onlyInApp", new Error("only in app"));
    return null;
  }
  return invoke<PracticeSession>(CMD.practiceScenarioStart, {
    practiceType,
    ...(targetNodeId ? { targetNodeId } : {}),
    ...(materialBookId ? { materialBookId } : {}),
  });
}

/** 评估本轮文本作答，得到 AI 引导反馈与评分。 */
export async function practiceScenarioEvaluate(
  sessionId: string,
  userOutput: string,
): Promise<PracticeEval | null> {
  if (!isTauri()) return null;
  return invoke<PracticeEval>(CMD.practiceScenarioEvaluate, {
    sessionId,
    userOutput,
  });
}

/** 拉取某会话全部评估记录（按时间正序）。 */
export async function practiceScenarioHistory(
  sessionId: string,
): Promise<PracticeEval[]> {
  if (!isTauri()) return [];
  return invoke<PracticeEval[]>(CMD.practiceScenarioHistory, { sessionId });
}

// ============ 语音问答 ============

/** 语音问答出题（可选 targetNodeId 定向知识点）。 */
export async function voicePracticeAsk(
  targetNodeId?: string | null,
  materialBookId?: string | null,
): Promise<VoiceAsk | null> {
  if (!isTauri()) {
    logError("practiceService.voiceAsk.onlyInApp", new Error("only in app"));
    return null;
  }
  return invoke<VoiceAsk>(CMD.voicePracticeAsk, {
    ...(targetNodeId ? { targetNodeId } : {}),
    ...(materialBookId ? { materialBookId } : {}),
  });
}

/** 语音作答（转写文本）→ 评分与反馈。 */
export async function voicePracticeAnswer(
  sessionId: string,
  transcribedText: string,
  userAudioPath?: string | null,
): Promise<VoiceAnswer | null> {
  if (!isTauri()) return null;
  return invoke<VoiceAnswer>(CMD.voicePracticeAnswer, {
    sessionId,
    transcribedText,
    ...(userAudioPath ? { userAudioPath } : {}),
  });
}

// ============ 教学相长 ============

/** 开启一次"AI 当学生"教学会话。 */
export async function teachingStart(
  targetKnowledgeId?: string | null,
  materialBookId?: string | null,
): Promise<TeachingSession | null> {
  if (!isTauri()) {
    logError("practiceService.teachingStart.onlyInApp", new Error("only in app"));
    return null;
  }
  return invoke<TeachingSession>(CMD.teachingStart, {
    ...(targetKnowledgeId ? { targetKnowledgeId } : {}),
    ...(materialBookId ? { materialBookId } : {}),
  });
}

/** 用户作答 → AI 追问；满 5 轮自动结课并出三围报告。 */
export async function teachingRespond(
  sessionId: string,
  userAnswer: string,
): Promise<TeachingSession | null> {
  if (!isTauri()) return null;
  return invoke<TeachingSession>(CMD.teachingRespond, {
    sessionId,
    userAnswer,
  });
}

/** 手动结束教学，产出清晰度/完整性/准确性三围评分 + 报告。 */
export async function teachingFinish(
  sessionId: string,
): Promise<TeachingSession | null> {
  if (!isTauri()) return null;
  return invoke<TeachingSession>(CMD.teachingFinish, { sessionId });
}

/** 教学历史（最近 50 个 session）。 */
export async function teachingHistory(): Promise<TeachingSession[]> {
  if (!isTauri()) return [];
  return invoke<TeachingSession[]>(CMD.teachingHistory);
}

// ============ 语音 AI 教练 ============

/** 新建一个语音教练会话。 */
export async function voiceCoachStart(): Promise<VoiceCoachSession | null> {
  if (!isTauri()) {
    logError("practiceService.coachStart.onlyInApp", new Error("only in app"));
    return null;
  }
  return invoke<VoiceCoachSession>(CMD.voiceCoachStart);
}

/** 输入用户转写文本，AI 结合历史给出回复。 */
export async function voiceCoachInput(
  sessionId: string,
  transcribedText: string,
): Promise<VoiceCoachReply | null> {
  if (!isTauri()) return null;
  return invoke<VoiceCoachReply>(CMD.voiceCoachInput, {
    sessionId,
    transcribedText,
  });
}

/** 打断当前 AI 播报（给最新 assistant 消息追加标记）。 */
export async function voiceCoachInterrupt(
  sessionId: string,
): Promise<VoiceCoachInterruptResult | null> {
  if (!isTauri()) return null;
  return invoke<VoiceCoachInterruptResult>(CMD.voiceCoachInterrupt, {
    sessionId,
  });
}

/** 读单个教练会话（含消息流）。 */
export async function voiceCoachSession(
  sessionId: string,
): Promise<VoiceCoachSession | null> {
  if (!isTauri()) return EMPTY_COACH_SESSION;
  return invoke<VoiceCoachSession | null>(CMD.voiceCoachSession, { sessionId });
}

/** 历史会话（最近 20 个）。 */
export async function voiceCoachHistory(): Promise<VoiceCoachSession[]> {
  if (!isTauri()) return [];
  return invoke<VoiceCoachSession[]>(CMD.voiceCoachHistory);
}