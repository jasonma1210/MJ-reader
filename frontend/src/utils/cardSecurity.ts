/**
 * 白板卡片安全白名单（cardSecurity.ts）
 * 用于两处：
 *  1. 提交「网页 / 在线视频」卡片时，白名单外的 URL 直接拦截并提示。
 *  2. 渲染阶段兜底：白名单外 URL 的 iframe 用「拦截占位」替代，避免库内脏数据/被改写的 embedding 页接管应用（对齐 FlexNote 1.1.46 修复）。
 *
 * 设计原则：
 *  - 内联 iframe（`web`）只允许「可信任的只读站点」，其它一律落地页拦截 + 新窗口打开。
 *  - `onlineVideo` 走专用播放器域，识别失败则回退为拦截占位。
 *  - 域名匹配用「子域白名单」，覆盖恶意相似域名后缀（如 evil-youtube.com 不被命中）。
 */

/** 可信任、可用 iframe 内嵌的只读型站点（按域名后缀匹配子域） */
export const TRUSTED_WEB_DOMAINS: readonly string[] = [
  // 编程 / 文档 / 问答
  "wikipedia.org",
  "github.com",
  "github.io",
  "npmjs.com",
  "gitee.com",
  "gitlab.com",
  "msdn.microsoft.com",
  "developer.mozilla.org", // mdn；因其 www 前缀单独列，避免放行任意 microsoft 子站
  "react.dev",
  "vuejs.org",
  "typescriptlang.org",
  "nodejs.org",
  "python.org",
  "rust-lang.org",
  "golang.org",
  "stackoverflow.com",
  "stackexchange.com",
  "cnblogs.com",
  "csdn.net", // 描述为通用知识站，含 blog.csdn.net
  "juejin.cn", // 掘金
  "segmentfault.com",
  "jianshu.com",
  "zhihu.com",
  "medium.com",
  "baidu.com", // 百度百科/经验等可读站点
  "bing.com",
  "docs.google.com", // 谷歌文档 iframe 由 Google 自身允许展示
  "notion.site",
  "w3schools.com",
  "runoob.com",
  "gitbook.com",
  "readthedocs.io",
  // 学术 / 出版
  "arxiv.org",
  "springer.com",
  "acm.org",
  "ieee.org",
];

/** 可嵌入 iframe 的视频平台（含其短链域） */
export const VIDEO_EMBED_DOMAINS: readonly string[] = [
  "youtube.com",
  "youtu.be",
  "bilibili.com",
  "b23.tv",
  "vimeo.com",
  "qq.com", // 腾讯视频
  "youku.com",
];

/**
 * 判断 hostname 是否为某域集合的白名单成员（子域匹配）。
 * 例：`www.youtube.com` 命中 `youtube.com`；`evil-youtube.com` 不命中。
 */
function matchesDomains(hostname: string, domains: readonly string[]): boolean {
  const host = hostname.toLowerCase().replace(/^www\./, "");
  return domains.some((d) => host === d || host.endsWith(`.${d}`));
}

/** 解析并返回 URL 的主机名小写；非法/非 http(s) 返回空串 */
export function getDisplayHost(raw: string): string {
  try {
    const u = new URL(raw.trim());
    if (u.protocol !== "http:" && u.protocol !== "https:") return "";
    return u.hostname.toLowerCase().replace(/^www\./, "");
  } catch {
    return "";
  }
}

/** 是否为合法 http(s) URL */
export function isHttpUrl(raw: string): boolean {
  try {
    const u = new URL(raw.trim());
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
}

/**
 * 卡片 URL 是否允许内联渲染。
 * @param kind 卡片类型：`web`（信任站白名单）或 `onlineVideo`（视频平台白名单，兜底要求能被 toEmbedUrl 识别为平台）。
 */
export function isUrlAllowed(raw: string, kind: "web" | "onlineVideo"): boolean {
  const host = getDisplayHost(raw);
  if (!host) return false;
  return kind === "web"
    ? matchesDomains(host, TRUSTED_WEB_DOMAINS)
    : matchesDomains(host, VIDEO_EMBED_DOMAINS);
}

/** 内联 `web` 卡片 iframe 的 sandbox 属性（不允许顶层导航，防嵌入页接管主应用） */
export const WEB_IFRAME_SANDBOX =
  "allow-scripts allow-same-origin allow-forms allow-popups";