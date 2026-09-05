/**
 * 列表行五色随机竖线（学习者闭环 · 区分每行笔记/高亮内容）：
 * - 色池固定为红 / 绿 / 黄 / 蓝 / 橙，排除白与黑；
 * - 相邻（连续）两条不允许同色——遍历时跳过与上一条相同的颜色。
 */

export const STRIPE_COLORS = [
  "#EF4444", // 红
  "#22C55E", // 绿
  "#EAB308", // 黄
  "#3B82F6", // 蓝
  "#F97316", // 橙
] as const;

/** 伪随机取一个与前一条不同色（cur 为当前序号，prev 为上一条颜色，返回新色） */
export function nextStripeColor(cur: number, prev?: string): string {
  if (cur === 0) {
    const first = STRIPE_COLORS[cur % STRIPE_COLORS.length];
    return first;
  }
  // 从色池中排除上一条颜色，再在剩余候选里取一个
  const pool = STRIPE_COLORS.filter((c) => c !== prev);
  const idx = (cur + Math.floor(Math.random() * pool.length)) % pool.length;
  return pool[idx];
}

/** 一次生成 count 条相邻不同色的竖线色值序列（i18n-safe，纯函数便于测试） */
export function buildStripeColors(count: number): string[] {
  const out: string[] = [];
  for (let i = 0; i < count; i++) {
    out.push(nextStripeColor(i, out[i - 1]));
  }
  return out;
}