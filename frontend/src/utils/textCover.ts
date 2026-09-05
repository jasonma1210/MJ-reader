/* MJNexus Reader — 无封面文档的首屏文字封面生成
 *
 * 需求：书架封面 = 书籍第一页内容；文档没有内嵌封面时，用第一页内容作为封面。
 * 对 PDF 而言第一页天然是内容页（pdf.js 已实现导出）；对 EPUB/Office/Text 这类
 * 以文字为主、且无法可靠做 DOM 截图的格式，统一把「书名 + 正文开头」绘制成
 * 纸面风格的首屏封面 PNG，再经后端 save_book_cover 落盘到 covers/{book_id}.png。
 *
 * 说明：
 * - 用户显式要求「全部格式全部实现」；本工具负责 EPUB/Office/Text 的兜底。
 * - 若书籍已有封面（内嵌/已生成），maybeSaveFirstPageCover 会跳过，遵守
 *   「有封面直接用封面」的规则。
 * - 书架加载时，对没有封面的书先生成「纯书名占位封面」
 *   （generateTitlePlaceholderCover，落盘 covers/{book_id}.placeholder.png）；
 *   打开书本后由首屏内容封面升级覆盖（maybeSaveFirstPageCover）。
 * - 采用手动 canvas 文字绘制而非 DOM 截图（html-to-image 等依赖 foreignObject，
 *   在 Android/iOS WebView 不可靠），跨端最稳；不依赖 lookbehind 正则（兼容旧 WebView）。
 */

import { invoke } from "@tauri-apps/api/core";
import { bookService } from "../services/bookService";
import { loadBookBytes } from "./bookFileLoader";
import { logError } from "./logError";

/** 本次会话已尝试过封面生成的书籍（避免每次打开/翻页重复落盘覆盖已有封面） */
const sessionCovered = new Set<string>();
/** 本次会话已生成过标题占位封面的书籍 */
const sessionPlaceholderCovered = new Set<string>();

const FONT_STACK =
  '-apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif';

/** 字节数组 → base64（分块避免大数组拼接导致调用栈溢出） */
function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + CHUNK)));
  }
  return btoa(binary);
}

/** 逐字断行（不含 lookbehind，兼容老版 Android System WebView） */
function wrapText(ctx: CanvasRenderingContext2D, text: string, maxWidth: number): string[] {
  const out: string[] = [];
  let line = "";
  for (const ch of text) {
    if (ch === "\n") {
      out.push(line);
      line = "";
      continue;
    }
    const next = line + ch;
    if (line && ctx.measureText(next).width > maxWidth) {
      out.push(line);
      line = ch;
    } else {
      line = next;
    }
  }
  if (line) out.push(line);
  return out;
}

/** 把「书名 + 正文开头」绘制成 3:4 纸面首屏封面，返回 PNG 字节。 */
export async function renderTextCover(title: string, text: string): Promise<Uint8Array> {
  const W = 1200;
  const H = 1600;
  const canvas = document.createElement("canvas");
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext("2d");
  if (!ctx) return new Uint8Array(0);

  const M = 96;
  const maxW = W - M * 2;

  // 纸面背景（书封观感：暖白纸 + 细边）
  ctx.fillStyle = "#f5f5f0";
  ctx.fillRect(0, 0, W, H);
  ctx.strokeStyle = "rgba(31,36,48,0.16)";
  ctx.lineWidth = 6;
  ctx.strokeRect(24, 24, W - 48, H - 48);

  let bodyTop = 150;

  // 书名（顶部，最多两行，居中）
  const titleClean = (title || "").trim();
  if (titleClean) {
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    ctx.fillStyle = "#1f2430";
    ctx.font = `700 62px ${FONT_STACK}`;
    const titleLines = wrapText(ctx, titleClean, maxW).slice(0, 2);
    let y = 150;
    for (const ln of titleLines) {
      ctx.fillText(ln, W / 2, y);
      y += 84;
    }
    // 分隔线
    const lineY = Math.max(y + 14, 360);
    ctx.strokeStyle = "rgba(31,36,48,0.25)";
    ctx.lineWidth = 3;
    ctx.beginPath();
    ctx.moveTo(W / 2 - 90, lineY);
    ctx.lineTo(W / 2 + 90, lineY);
    ctx.stroke();
    bodyTop = lineY + 56;
  }

  // 正文开头：去多余空白后回行绘制，超出画布即截断（呈现“第一页”）
  const body = (text || "").replace(/\s+/g, " ").trim();
  if (body) {
    ctx.textAlign = "left";
    ctx.textBaseline = "top";
    ctx.fillStyle = "#0f172a";
    ctx.font = `400 28px ${FONT_STACK}`;
    const lh = 46;
    const lines = wrapText(ctx, body, maxW);
    let y = bodyTop;
    for (const ln of lines) {
      if (y + lh > H - M - 24) break;
      ctx.fillText(ln, M, y);
      y += lh;
    }
  }

  return canvasToPngBytes(canvas);
}

/** HTMLCanvasElement → PNG 字节（toBlob 兼容性最好） */
function canvasToPngBytes(canvas: HTMLCanvasElement): Promise<Uint8Array> {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (!blob) {
        reject(new Error("canvas.toBlob 为空"));
        return;
      }
      blob
        .arrayBuffer()
        .then((buf) => resolve(new Uint8Array(buf)))
        .catch(reject);
    }, "image/png");
  });
}

/**
 * 判定一个 coverPath 是否为「标题占位封面」（书架加载时生成的纯书名封面）。
 * 打开书本后需用首屏内容封面升级覆盖占位封面。
 */
export function isPlaceholderCoverPath(coverPath: string | null | undefined): boolean {
  return !!coverPath && /\.placeholder\.png$/i.test(coverPath);
}

/**
 * 书架加载时生成「标题占位封面」：仅画书名、不画正文的纸面封面。
 * 规则：
 * 1. 本次会话已生成过该书 → 跳过。
 * 2. 书籍已有封面（coverPath 非空，含内嵌封面/正式封面/已有占位）→ 跳过。
 * 3. 书籍没有标题 → 跳过。
 * 4. 绘制纯书名 PNG，经后端 save_book_cover(placeholder=true) 落盘到
 *    covers/{book_id}.placeholder.png（与正式首屏封面区分，打开后便于升级覆盖）。
 * 返回是否真的生成了占位封面。
 */
export async function generateTitlePlaceholderCover(
  bookId: string,
  fallbackTitle = "",
): Promise<boolean> {
  if (!bookId || sessionPlaceholderCovered.has(bookId)) return false;
  sessionPlaceholderCovered.add(bookId);
  try {
    const book = await bookService.getBookById(bookId);
    if (!book) return false;
    if (book.coverPath && String(book.coverPath).trim()) return false; // 已有封面 → 无需占位
    const title = (book.title || fallbackTitle || "").trim();
    if (!title) return false; // 连标题都没有则放弃
    const bytes = await renderTextCover(title, "");
    if (bytes.length === 0) return false;
    // 后端 save_book_cover 形参为 snake_case：book_id / image_data / placeholder
    await invoke("save_book_cover", {
      bookId,
      imageData: bytesToBase64(bytes),
      placeholder: true,
    });
    return true;
  } catch (e) {
    logError("textCover.placeholder", e);
    return false;
  }
}

/**
 * 无封面时保存「首屏文字封面」。规则：
 * 1. 本次会话已处理过该书 → 跳过（避免封面反复覆盖）。
 * 2. 书籍已有「非占位」封面（内嵌封面/已生成的正式封面）且可读取 → 跳过；
 *    占位封面 / 陈旧不可读封面 → 落到下方升级覆盖为首屏内容封面。
 * 3. 取渲染内容的开头文本，绘制 PNG 并入库（覆盖 covers/{book_id}.placeholder.png）。
 */
export async function maybeSaveFirstPageCover(
  bookId: string,
  text: string,
  fallbackTitle = "",
): Promise<void> {
  if (!bookId || sessionCovered.has(bookId)) return;
  sessionCovered.add(bookId); // 先占位，防止并发重复写入
  try {
    const book = await bookService.getBookById(bookId);
    if (!book) return;
    // 已有正式封面（非占位）且可读取才保留；占位/陈旧封面落到下方升级生成。
    if (book.coverPath && String(book.coverPath).trim() && !isPlaceholderCoverPath(book.coverPath)) {
      try {
        const { bytes } = await loadBookBytes(book.coverPath);
        if (bytes.length > 0) return;
      } catch (e) {
        // 读不到 → 视为陈旧封面，落到下方重新生成
        logError("textCover.book", e);
      }
    }
    const title = (book.title || fallbackTitle || "").trim();
    const body = (text || "").replace(/\s+/g, " ").trim();
    if (!title && !body) return; // 连标题都没有则放弃
    const bytes = await renderTextCover(title, body);
    if (bytes.length === 0) return;
    // 后端 save_book_cover 形参为 snake_case：book_id / image_data（base64，规避移动端大数组 IPC）
    await invoke("save_book_cover", {
      bookId,
      imageData: bytesToBase64(bytes),
    });
  } catch (e) {
    logError("textCover.saveFirstPage", e);
  }
}