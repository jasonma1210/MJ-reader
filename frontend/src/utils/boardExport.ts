import { toPng } from "html-to-image";
import { jsPDF } from "jspdf";
import { logError } from "./logError";

/**
 * 画布导出（计划 v1.1 M6）。
 * 策略：对画布可视区容器截图（所见即所得），包含卡片/连线/图元/收纳组。
 * - PNG：html-to-image 渲染当前视口 → dataURL。
 * - PDF：先把 PNG 嵌入 A4 页面（整板当前视图单页）。
 * 兼容性：依赖浏览器对 CSS 变量(var(--color-*)) 的注入计算；WebView 截图失败时回退 try/catch 由调用方 toast。
 */

/** 截图时临时注入的高亮背景/边框，避免导出为透明底 */
// eslint-disable-next-line @typescript-eslint/no-unused-vars
const EXPORT_FILL = "#ffffff";

/**
 * html-to-image 会把被截图的 DOM 克隆进内部 iframe 后绘制到 canvas。
 * 若 `<img>` 仍指向 Tauri asset 协议 / blob 等跨源地址，绘制时会污染 canvas，
 * 导致 toDataURL 抛 SecurityError（导出必失败）。因此在截图前把这类图片临时
 * 替换为 dataURL，截图完成后恢复原图，避免污染画布。
 * 返回一个恢复函数；内联失败的图片保持原值（加载失败的空图不会污染 canvas）。
 */
function inlineExternalImages(root: HTMLElement): { ready: Promise<void>; restore: () => void } {
  const toRestore: Array<[HTMLImageElement, string]> = [];
  const images = Array.from(root.querySelectorAll("img")).filter((img) => {
    const src = img.currentSrc || img.getAttribute("src") || "";
    return /^(asset:|blob:)/.test(src);
  });
  for (const img of images) {
    const original = img.currentSrc || img.getAttribute("src") || "";
    toRestore.push([img, original]);
  }
  const ready = Promise.all(
    toRestore.map(async ([img, original]) => {
      try {
        const resp = await fetch(original);
        if (!resp.ok) return;
        const blob = await resp.blob();
        img.setAttribute("src", URL.createObjectURL(blob));
      } catch (e) {
        logError("boardExport.restoreRemoteImage", e);
      }
    }),
  ).then(() => undefined);
  return {
    ready,
    restore: () => {
      for (const [img, original] of toRestore) img.setAttribute("src", original);
    },
  };
}

async function renderDataUrl(target: HTMLElement, background: string): Promise<string> {
  // 先内联跨源图片，记录恢复函数；等内联完成后截图，避免 asset/blob 图污染画布
  const { ready, restore } = inlineExternalImages(target);
  try {
    await ready;
    return await toPng(target, {
      pixelRatio: 2,
      backgroundColor: background,
      cacheBust: true,
      /** 跳过字体内嵌：避免跨域字体抓取失败导致每次导出都 reject */
      skipFonts: true,
      width: target.offsetWidth,
      height: target.offsetHeight,
    });
  } catch (e) {
    throw new Error(`PNG render failed: ${String((e as Error)?.message ?? e)}`);
  } finally {
    restore();
  }
}

/** 导出当前画布视图为 PNG，返回 dataURL */
export async function exportBoardPng(target: HTMLElement, background = "#ffffff"): Promise<string> {
  const url = await renderDataUrl(target, background);
  if (typeof document === "undefined") return url;
  const a = document.createElement("a");
  a.href = url;
  a.download = `whiteboard-${Date.now()}.png`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  return url;
}

/** 导出当前画布视图为 PDF（png 嵌入 A4 单页，横向铺满） */
export async function exportBoardPdf(target: HTMLElement, background = "#ffffff"): Promise<string> {
  const url = await renderDataUrl(target, background);
  const pdf = new jsPDF({
    orientation: "landscape",
    unit: "mm",
    format: "a4",
  });
  const pageW = pdf.internal.pageSize.getWidth();
  const pageH = pdf.internal.pageSize.getHeight();
  // 图片填充整页，按比例缩放居中
  pdf.addImage(url, "PNG", 0, 0, pageW, pageH, undefined, "FAST");
  const filename = `whiteboard-${Date.now()}.pdf`;
  pdf.save(filename);
  return filename;
}