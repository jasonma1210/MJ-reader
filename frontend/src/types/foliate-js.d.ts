/**
 * foliate-js 子模块类型声明
 *
 * foliate-js 1.0.1 不带 .d.ts，package.json 的 exports 为 `"./*.js": "./*.js"`，
 * 因此每个子模块都要单独声明。这里只声明我们真正调用到的最小面。
 */

declare module "foliate-js/view.js" {
  const _default: unknown;
  export default _default;
}

/** zip.js（foliate 内置 vendor 版本，API 与 @zip.js/zip.js 一致） */
declare module "foliate-js/vendor/zip.js" {
  export function configure(options: { useWebWorkers?: boolean }): void;
  export class BlobReader {
    constructor(blob: Blob);
  }
  export class TextWriter {
    constructor(encoding?: string);
  }
  export class BlobWriter {
    constructor(contentType?: string);
  }
  export interface ZipEntry {
    filename: string;
    uncompressedSize: number;
    getData(writer: unknown): Promise<never>;
  }
  export class ZipReader {
    constructor(reader: unknown);
    getEntries(): Promise<ZipEntry[]>;
  }
}

/** fflate（MOBI 的 palmdoc/huffcdic 解压依赖） */
declare module "foliate-js/vendor/fflate.js" {
  export function unzlibSync(data: Uint8Array): Uint8Array;
}

declare module "foliate-js/epub.js" {
  export class EPUB {
    constructor(loader: unknown);
    init(): Promise<unknown>;
  }
}

declare module "foliate-js/mobi.js" {
  export function isMOBI(file: Blob): Promise<boolean>;
  export class MOBI {
    constructor(options: { unzlib: (data: Uint8Array) => Uint8Array });
    open(file: Blob): Promise<unknown>;
  }
}

declare module "foliate-js/fb2.js" {
  export function makeFB2(blob: Blob): Promise<unknown>;
}

declare module "foliate-js/comic-book.js" {
  export function makeComicBook(loader: unknown, file: Blob): unknown;
}

declare module "foliate-js/overlayer.js" {
  export interface OverlayerRect {
    left: number;
    top: number;
    width: number;
    height: number;
  }
  export type DrawFunc = (
    rects: OverlayerRect[],
    options?: Record<string, unknown>
  ) => SVGElement;
  export class Overlayer {
    static highlight: DrawFunc;
    static underline: DrawFunc;
    static squiggly: DrawFunc;
    static outline: DrawFunc;
  }
}
