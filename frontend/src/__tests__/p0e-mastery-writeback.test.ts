/**
 * P0-E 复习回写去二值化单测：
 *  ① 四档 rating → 掌握度变化方向（后端 boolean 契约下的诚实降级）
 *  ② 无 bookId / 无 nodeId 时产出 issue，并实际触发 logError + toast
 *  ③ 教学结课 correct 由三围评分推导（非恒 true）
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../utils/toast", () => ({ toast: vi.fn() }));
vi.mock("../utils/logError", () => ({ logError: vi.fn() }));

import { toast } from "../utils/toast";
import { logError } from "../utils/logError";
import {
  ratingToMasteryDirection,
  planMasteryWriteback,
  reportMasteryWritebackIssue,
  isTeachingMasteryCorrect,
  TEACHING_PASS_SCORE,
} from "../services/closedLoopService";

const mockedToast = vi.mocked(toast);
const mockedLogError = vi.mocked(logError);

beforeEach(() => {
  vi.clearAllMocks();
});

describe("① 四档评分 → 掌握度方向（去二值化映射）", () => {
  it("again 降；hard / good / easy 升（后端 boolean 契约无法表达 hard 的「平」）", () => {
    expect(ratingToMasteryDirection("again")).toBe("down");
    expect(ratingToMasteryDirection("hard")).toBe("up");
    expect(ratingToMasteryDirection("good")).toBe("up");
    expect(ratingToMasteryDirection("easy")).toBe("up");
  });

  it("未知评分按 good 兜底，与后端 record_card_review 默认 quality=4 一致", () => {
    expect(ratingToMasteryDirection("unknown")).toBe("up");
  });

  it("plan 把方向转成 correct：again=false，其余=true", () => {
    expect(planMasteryWriteback("b1", "n1", "again")).toMatchObject({
      kind: "write",
      correct: false,
    });
    for (const rating of ["hard", "good", "easy"]) {
      expect(planMasteryWriteback("b1", "n1", rating)).toMatchObject({
        kind: "write",
        correct: true,
      });
    }
  });
});

describe("② 关联失败有日志不静默（logError + UI 提示）", () => {
  it("无 bookId → issue 且 logError + toast 均被调用", () => {
    const plan = planMasteryWriteback(null, "n1", "good");
    expect(plan.kind).toBe("issue");
    if (plan.kind !== "issue") return;
    expect(plan.reason).toBe("no-book");

    // 不传 deps → 使用真实默认依赖（已被 mock），验证生产接线
    reportMasteryWritebackIssue(plan, (key) => `i18n:${key}`);
    expect(mockedLogError).toHaveBeenCalledTimes(1);
    expect(mockedToast).toHaveBeenCalledTimes(1);
    expect(mockedToast).toHaveBeenCalledWith("i18n:review.masteryNoBook");
  });

  it("无 nodeId → issue 且 logError + toast 均被调用（不同 i18n key）", () => {
    const plan = planMasteryWriteback("b1", undefined, "good");
    expect(plan.kind).toBe("issue");
    if (plan.kind !== "issue") return;
    expect(plan.reason).toBe("no-node");

    // 注入 deps 验证可断言性
    const log = vi.fn();
    const show = vi.fn();
    reportMasteryWritebackIssue(plan, (key) => `i18n:${key}`, {
      logError: log,
      toast: show,
    });
    expect(log).toHaveBeenCalledTimes(1);
    expect(show).toHaveBeenCalledWith("i18n:review.masteryNoNode");
  });

  it("bookId + nodeId 齐全 → write，不触发任何通知", () => {
    const plan = planMasteryWriteback("b1", "n1", "again");
    expect(plan).toMatchObject({ kind: "write", bookId: "b1", nodeId: "n1" });
    expect(mockedLogError).not.toHaveBeenCalled();
    expect(mockedToast).not.toHaveBeenCalled();
  });
});

describe("③ 教学结课 correct 由三围评分推导（非恒 true）", () => {
  it("高分结课 → true，低分结课 → false", () => {
    expect(
      isTeachingMasteryCorrect({
        clarityScore: 90,
        completenessScore: 85,
        accuracyScore: 95,
      }),
    ).toBe(true);
    expect(
      isTeachingMasteryCorrect({
        clarityScore: 20,
        completenessScore: 30,
        accuracyScore: 10,
      }),
    ).toBe(false);
  });

  it("阈值边界：三围均值等于 60 视为通过，低于 60 视为未掌握", () => {
    expect(TEACHING_PASS_SCORE).toBe(60);
    expect(
      isTeachingMasteryCorrect({
        clarityScore: 60,
        completenessScore: 60,
        accuracyScore: 60,
      }),
    ).toBe(true);
    expect(
      isTeachingMasteryCorrect({
        clarityScore: 59,
        completenessScore: 60,
        accuracyScore: 60,
      }),
    ).toBe(false);
  });

  it("三围等权：accuracy 显著偏低可翻转结论", () => {
    // 另两维 70/70 本已达标（均值 70）；accuracy=10 把均值拉到 50 → 未掌握
    expect(
      isTeachingMasteryCorrect({
        clarityScore: 70,
        completenessScore: 70,
        accuracyScore: 70,
      }),
    ).toBe(true);
    expect(
      isTeachingMasteryCorrect({
        clarityScore: 70,
        completenessScore: 70,
        accuracyScore: 10,
      }),
    ).toBe(false);
  });
});
