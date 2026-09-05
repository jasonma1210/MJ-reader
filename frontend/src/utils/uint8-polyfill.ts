// pdfjs-dist 6.x 在多个地方调用 ES2024 提案方法（移植自 frontend-deprecated）：
// Uint8Array.prototype.toHex / Map.getOrInsertComputed / WeakMap.getOrInsertComputed
// 部分 Android WebView 缺失，导致 PDF 打开失败（UnknownErrorException: a.toHex is not a function）。
// worker 与主线程启动时各注入一次。

declare global {
  interface Uint8Array {
    toHex?: () => string;
  }
  interface Map<K, V> {
    getOrInsertComputed?: (key: K, callback: (key: K) => V) => V;
  }
  interface WeakMap<K extends WeakKey, V> {
    getOrInsertComputed?: (key: K, callback: (key: K) => V) => V;
  }
}

if (typeof (Uint8Array.prototype as Uint8Array).toHex !== "function") {
  Object.defineProperty(Uint8Array.prototype, "toHex", {
    value: function toHex(this: Uint8Array): string {
      let out = "";
      const len = this.length;
      for (let i = 0; i < len; i++) {
        out += this[i].toString(16).padStart(2, "0");
      }
      return out;
    },
    configurable: true,
    writable: true,
  });
}

if (typeof (Map.prototype as Map<unknown, unknown>).getOrInsertComputed !== "function") {
  Object.defineProperty(Map.prototype, "getOrInsertComputed", {
    value: function getOrInsertComputed<K, V>(
      this: Map<K, V>,
      key: K,
      callback: (key: K) => V,
    ): V {
      let value = this.get(key);
      if (value === undefined) {
        value = callback(key);
        this.set(key, value);
      }
      return value;
    },
    configurable: true,
    writable: true,
  });
}

if (typeof (WeakMap.prototype as WeakMap<object, unknown>).getOrInsertComputed !== "function") {
  Object.defineProperty(WeakMap.prototype, "getOrInsertComputed", {
    value: function getOrInsertComputed<K extends object, V>(
      this: WeakMap<K, V>,
      key: K,
      callback: (key: K) => V,
    ): V {
      let value = this.get(key);
      if (value === undefined) {
        value = callback(key);
        this.set(key, value);
      }
      return value;
    },
    configurable: true,
    writable: true,
  });
}

export {};
