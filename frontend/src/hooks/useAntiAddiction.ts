import { useEffect, useRef, useState } from "react";
import { isTauri } from "../services/tauri";
import { statsService } from "../services/statsService";
import { logError } from "../utils/logError";
import {
  currentAgeMode,
  effectiveDailyLimit,
  useAgeStore,
} from "../stores/ageStore";
import { continuousReminderMinutes } from "../services/ageGuard";

/**
 * 防沉迷（A3）：单日使用时长上限 + 连续使用提醒。
 *
 * better-harness：策略判定全部走 ageGuard.ts / ageStore.ts 单一真源，本 hook 只做
 * 「读取今日真实阅读时长（复用 statsService.getStats().todaySeconds）→ 与生效上限比较」。
 * fail-closed：受限档（child/teen）默认锁定，须家长 PIN 才解锁；PIN 错误保持锁定。
 *
 * 注：今日时长来自后端 reading_stats（与学习中心/热力图同源），非本地自计时，避免被绕过。
 */

const REFRESH_MS = 30_000;
/** 家长 PIN（演示默认；生产应改为家长端设置并本地加密存储，fail-closed 默认锁定） */
const PARENT_PIN = "0000";
/** 一次解锁的有效时长（毫秒） */
const UNLOCK_WINDOW_MS = 30 * 60 * 1000;
/** 连续使用提醒的静默冷却（毫秒），避免频繁弹窗 */
const REMINDER_COOLDOWN_MS = 30 * 60 * 1000;

export interface AntiAddictionState {
  /** 生效的单日上限（分钟）；null = 不限 */
  limitMinutes: number | null;
  /** 今日已用（秒） */
  usedSeconds: number;
  /** 剩余（秒）；不限为 Infinity */
  remainingSeconds: number;
  /** 已达上限并锁定 */
  isLocked: boolean;
  /** 连续使用提醒（未锁定时触发，非阻塞） */
  reminderDue: boolean;
  /** 忽略本次提醒 */
  dismissReminder: () => void;
  /** 家长 PIN 解锁（fail-closed：仅正确 PIN 返回 true 并开放一个窗口期） */
  parentUnlock: (code: string) => boolean;
}

export function useAntiAddiction(): AntiAddictionState {
  const mode = currentAgeMode();
  const overrides = useAgeStore((s) => s.limitOverrides);
  const limit = effectiveDailyLimit(mode);
  const reminderThreshold = continuousReminderMinutes(mode);

  const [usedSeconds, setUsedSeconds] = useState(0);
  const [lastReminderDismiss, setLastReminderDismiss] = useState(0);
  const [lockedUntil, setLockedUntil] = useState<number | null>(null);
  const lockedUntilRef = useRef<number | null>(null);
  lockedUntilRef.current = lockedUntil;

  const fetchUsed = () => {
    if (!isTauri()) return;
    statsService
      .getStats()
      .then((s) => setUsedSeconds(s.todaySeconds))
      .catch((e) => logError("useAntiAddiction.fetchUsed", e));
  };

  useEffect(() => {
    fetchUsed();
    const iv = window.setInterval(fetchUsed, REFRESH_MS);
    const onFocus = () => fetchUsed();
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearInterval(iv);
      window.removeEventListener("focus", onFocus);
    };
    // overrides 变化也要重算（家长调上限后立即反映）
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, JSON.stringify(overrides)]);

  const limitSec = limit == null ? Infinity : limit * 60;
  const stillUnlocked =
    lockedUntilRef.current != null && Date.now() < lockedUntilRef.current;
  const isLocked =
    limit != null && usedSeconds >= limitSec && !stillUnlocked;

  const reminderDue =
    limit != null &&
    !isLocked &&
    reminderThreshold != null &&
    usedSeconds >= reminderThreshold * 60 &&
    Date.now() - lastReminderDismiss > REMINDER_COOLDOWN_MS;

  return {
    limitMinutes: limit,
    usedSeconds,
    remainingSeconds:
      limit == null ? Infinity : Math.max(0, limitSec - usedSeconds),
    isLocked,
    reminderDue,
    dismissReminder: () => setLastReminderDismiss(Date.now()),
    parentUnlock: (code: string) => {
      if (code === PARENT_PIN) {
        setLockedUntil(Date.now() + UNLOCK_WINDOW_MS);
        return true;
      }
      return false;
    },
  };
}
