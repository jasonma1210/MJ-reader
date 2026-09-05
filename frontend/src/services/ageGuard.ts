/**
 * 适龄分级护栏（better-harness：共享组件复用 + fail-closed 守卫）。
 *
 * 审计报告 A1/A2（P0）根因：产品定位「全年龄段」，却无年龄分级、无家长/AI 护栏，
 * 儿童可任意联网检索、AI 对话对未成年人无敏感话题过滤。
 *
 * 本模块是护栏的「单一真源」：所有页面/服务关于适龄的决策都必须经此，
 * 不得在各处手写 `if (child) ...`。JOIN 与守卫被打包成不可分割的纯函数，
 * 即使调用方忘记判断，受限档（child/teen）默认关闭网络、开启护栏——
 * 即「fail-closed：默认最严，而非默认放开」。
 */

export type AgeMode = "child" | "teen" | "adult";

export interface AgeTierPolicy {
  /** 是否允许联网检索 / 网络搜索（A1：儿童/青少年关闭） */
  networkImportAllowed: boolean;
  /** 是否启用 AI 内容护栏（A2：儿童/青少年启用） */
  contentGuardEnabled: boolean;
  /** 是否启用简化 UI（A1：仅儿童） */
  uiSimplified: boolean;
  /** 单日使用时长上限（分钟）；null = 不限（A3 防沉迷：仅 child/teen 设上限） */
  dailyLimitMinutes: number | null;
  /** 连续使用提醒阈值（分钟）；null = 不提醒（A3） */
  continuousReminderMinutes: number | null;
  /** i18n 标签键 */
  labelKey: string;
}

/**
 * 三档策略表（fail-closed 默认最严）。
 * 注意：adult 档 networkImportAllowed=true、contentGuardEnabled=false，
 * 仅代表「不额外施加客户端护栏」；产品是否对成人也做基础安全过滤属产品决策，不在此模块强制。
 */
export const AGE_TIERS: Record<AgeMode, AgeTierPolicy> = {
  child: {
    networkImportAllowed: false,
    contentGuardEnabled: true,
    uiSimplified: true,
    dailyLimitMinutes: 40,
    continuousReminderMinutes: 30,
    labelKey: "me.ageMode.child",
  },
  teen: {
    networkImportAllowed: false,
    contentGuardEnabled: true,
    uiSimplified: false,
    dailyLimitMinutes: 90,
    continuousReminderMinutes: 60,
    labelKey: "me.ageMode.teen",
  },
  adult: {
    networkImportAllowed: true,
    contentGuardEnabled: false,
    uiSimplified: false,
    dailyLimitMinutes: null,
    continuousReminderMinutes: null,
    labelKey: "me.ageMode.adult",
  },
};

/** 是否允许联网检索 / 网络搜索（A1） */
export function networkImportAllowed(mode: AgeMode): boolean {
  return AGE_TIERS[mode].networkImportAllowed;
}

/** 是否启用 AI 内容护栏（A2） */
export function contentGuardEnabled(mode: AgeMode): boolean {
  return AGE_TIERS[mode].contentGuardEnabled;
}

/** 是否启用简化 UI（A1） */
export function uiSimplified(mode: AgeMode): boolean {
  return AGE_TIERS[mode].uiSimplified;
}

/** 单日使用时长上限（分钟）；null = 不限（A3 防沉迷） */
export function dailyLimitMinutes(mode: AgeMode): number | null {
  return AGE_TIERS[mode].dailyLimitMinutes;
}

/** 连续使用提醒阈值（分钟）；null = 不提醒（A3） */
export function continuousReminderMinutes(mode: AgeMode): number | null {
  return AGE_TIERS[mode].continuousReminderMinutes;
}

/**
 * 年龄档系统护栏指令（fail-closed：儿童/青少年档非空前置，强制年龄适配语气 + 敏感话题拒答）。
 * adult 档返回空串（不施加限制）。该指令在 chatStream 调用前被前置为 system 消息，
 * 即使后端忽略也由客户端强制约束。
 *
 * 内容安全基线参考 COPPA / 未成年保护通用实践；具体分级阈值（如 teen 上限年龄）
 * 属产品决策，由产品负责人在统一评审中最终敲定。
 */
export function buildAgeAwareSystemInstruction(mode: AgeMode): string {
  if (mode === "adult") return "";
  const isChild = mode === "child";
  const audience = isChild ? "儿童（K12 及以下）" : "青少年";
  return [
    `[内容安全护栏 · ${audience}]`,
    `你正在为${audience}提供学习辅助。必须遵守以下规则：`,
    "1. 使用适合该年龄层、积极、鼓励性的语言；绝不生成暴力、色情、自残、吸毒、违法或恐怖相关内容。",
    "2. 若用户主动询问上述敏感话题，不得展开，须明确且温和地拒绝，并引导其向家长或老师求助。",
    "3. 不协助完成家庭作业或考试作弊；鼓励独立思考与学习方法。",
    "4. 不收集、不诱导透露姓名、学校、家庭住址、电话等个人身份信息（PII）。",
    isChild
      ? "5. 内容须浅显、具引导性；遇到超出年龄理解范围的话题，建议由家长陪同阅读。"
      : "",
  ]
    .filter(Boolean)
    .join("\n");
}
