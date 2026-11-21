#!/usr/bin/env bash
# 确保 Android 目录批量扫描导入能力可用（2026-11-20）。
#
# 背景：
#   - src-tauri/gen/ 整目录不入库（.gitignore），tauri android init 会重新生成
#     MainActivity.kt / AndroidManifest.xml，手写 Kotlin 桥会被覆盖丢失。
#   - tauri-plugin-dialog 2.7.1 在 #[cfg(mobile)] 下明确返回 FolderPickerNotImplemented
#     （commands.rs:186），移动端无法用 dialog.open({directory:true}) 选目录。
#   - 因此需要一个 Kotlin object 单例桥：用系统 ACTION_OPEN_DOCUMENT_TREE 选目录，
#     经 DocumentsContract 递归枚举子文档 URI，通过 WebView.evaluateJavascript 回传前端。
#
# 本脚本幂等：Kotlin 桥文件存在且 MainActivity 已注入则不动。
# 用法：tauri android init 之后、构建之前运行：
#   bash scripts/ensure-android-folder-picker.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GEN="$ROOT/src-tauri/gen/android/app/src/main/java/com/mjnexusreader/app"
MAIN="$GEN/MainActivity.kt"
BRIDGE="$GEN/FolderPickerBridge.kt"

mkdir -p "$GEN"

# ── 1. 写 Kotlin 桥（object 单例，JNI 可静态调用）──────────────────────────
if [ ! -f "$BRIDGE" ]; then
cat > "$BRIDGE" << 'KT'
package com.mjnexusreader.app

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import android.util.Log
import android.webkit.WebView
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import org.json.JSONArray

/**
 * Android SAF 目录批量扫描导入桥（2026-11-20）。
 *
 * 由 Rust 侧 JNI 命令 pick_android_folder 调 pick() 触发目录选择；
 * 用户选定目录后递归枚举子文档 URI，经 WebView.evaluateJavascript
 * 调前端全局回调 window.__mjnFolderPicked(jsonArray) 返回结果。
 *
 * 注意 registerForActivityResult 必须在 Activity STARTED 之前注册，
 * 故在 MainActivity.onCreate 中调用 register(activity)，launcher 存在本单例。
 */
object FolderPickerBridge {
    private const val TAG = "MjnFolderBridge"
    private var launcher: ActivityResultLauncher<Uri?>? = null
    private var appContext: Context? = null
    private var webView: WebView? = null
    @Volatile private var pending = false

    /** 在 MainActivity.onCreate（super.onCreate 之后）调用，注册目录选择器。 */
    @JvmStatic
    fun register(activity: ComponentActivity) {
        appContext = activity.applicationContext
        if (launcher == null) {
            launcher = activity.registerForActivityResult(
                ActivityResultContracts.OpenDocumentTree()
            ) { uri: Uri? -> handleResult(uri) }
            Log.i(TAG, "register: OpenDocumentTree launcher created")
        } else {
            Log.i(TAG, "register: launcher already exists")
        }
    }

    /**
     * 暴露给 WebView JS 的桥对象。前端直接调 window.MjnFolderBridge.pick()
     * 触发系统目录选择，完全绕开 Rust JNI —— Tauri 2 下 ndk_context 未初始化，
     * JavaVM 无法从 Rust 侧获取（ASR/下载保活两个旧 JNI 桥也因此长期失效）。
     */
    // Kotlin object 是单例、无外部实例，不能声明 inner class；
    // 直接用普通嵌套类，通过 FolderPickerBridge. 显式访问单例方法。
    class JsBridge {
        @android.webkit.JavascriptInterface
        fun pick(): Boolean = FolderPickerBridge.pick()

        @android.webkit.JavascriptInterface
        fun isReady(): Boolean = FolderPickerBridge.isReady()
    }

    /** 在 WryActivity.onWebViewCreate 回调中调用：保存 WebView + 注入 JS 桥。 */
    @JvmStatic
    fun attachWebView(wv: WebView) {
        webView = wv
        // addJavascriptInterface 必须在页面加载前注入；onWebViewCreate 正是该时机。
        wv.addJavascriptInterface(JsBridge(), "MjnFolderBridge")
        Log.i(TAG, "attachWebView: JS bridge MjnFolderBridge injected")
    }

    /** 供 JS 桥调用：触发系统目录选择。返回是否已注册 launcher。 */
    @JvmStatic
    fun pick(): Boolean {
        val l = launcher
        Log.i(TAG, "pick() called: launcher=${l != null}, pending=$pending, webView=${webView != null}")
        if (l == null) return false
        if (pending) return true
        pending = true
        l.launch(null)
        return true
    }

    /** 前端主动查询：目录选择能力是否已就绪。 */
    @JvmStatic
    fun isReady(): Boolean {
        val ready = launcher != null && webView != null
        Log.i(TAG, "isReady() called: $ready")
        return ready
    }

    private fun handleResult(treeUri: Uri?) {
        Log.i(TAG, "handleResult: treeUri=$treeUri")
        if (treeUri == null) {
            emit("[]") // 用户取消 → 空数组
            pending = false
            return
        }
        // 持久化 URI 权限，避免授权在进程重启后失效（SAF 目录引用需要）。
        try {
            appContext?.contentResolver?.takePersistableUriPermission(
                treeUri, Intent.FLAG_GRANT_READ_URI_PERMISSION
            )
        } catch (_: SecurityException) {
            // 部分 ROM 不支持持久化授权；单次选择仍可用，仅不跨重启。
        }
        val uris = JSONArray()
        enumerateTree(treeUri, 0, uris)
        emit(uris.toString())
        pending = false
    }

    /** 递归枚举 tree 下的文档 URI（深度 ≤10，跳过非文档节点）。 */
    private fun enumerateTree(treeUri: Uri, depth: Int, out: JSONArray) {
        val ctx = appContext ?: return
        if (depth > 10) return
        val rootId = try {
            DocumentsContract.getTreeDocumentId(treeUri)
        } catch (_: IllegalArgumentException) {
            return
        }
        val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, rootId)
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
        )
        val cursor = try {
            ctx.contentResolver.query(childrenUri, projection, null, null, null)
        } catch (_: Exception) {
            null
        } ?: return
        cursor.use { c ->
            val idIdx = c.getColumnIndex(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
            val mimeIdx = c.getColumnIndex(DocumentsContract.Document.COLUMN_MIME_TYPE)
            if (idIdx < 0) return
            while (c.moveToNext()) {
                val docId = c.getString(idIdx) ?: continue
                val mime = if (mimeIdx >= 0) c.getString(mimeIdx) else null
                if (mime == DocumentsContract.Document.MIME_TYPE_DIR) {
                    // 子目录：以该 documentId 为根继续递归。
                    enumerateTree(treeUri, docId, depth + 1, out)
                } else {
                    out.put(DocumentsContract.buildDocumentUriUsingTree(treeUri, docId).toString())
                }
            }
        }
    }

    /** 递归重载：以指定 documentId 为根。 */
    private fun enumerateTree(treeUri: Uri, parentDocId: String, depth: Int, out: JSONArray) {
        if (depth > 10) return
        val ctx = appContext ?: return
        val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, parentDocId)
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
        )
        val cursor = try {
            ctx.contentResolver.query(childrenUri, projection, null, null, null)
        } catch (_: Exception) {
            null
        } ?: return
        cursor.use { c ->
            val idIdx = c.getColumnIndex(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
            val mimeIdx = c.getColumnIndex(DocumentsContract.Document.COLUMN_MIME_TYPE)
            if (idIdx < 0) return
            while (c.moveToNext()) {
                val docId = c.getString(idIdx) ?: continue
                val mime = if (mimeIdx >= 0) c.getString(mimeIdx) else null
                if (mime == DocumentsContract.Document.MIME_TYPE_DIR) {
                    enumerateTree(treeUri, docId, depth + 1, out)
                } else {
                    out.put(DocumentsContract.buildDocumentUriUsingTree(treeUri, docId).toString())
                }
            }
        }
    }

    private fun emit(json: String) {
        val wv = webView ?: return
        wv.post {
            wv.evaluateJavascript(
                "window.__mjnFolderPicked && window.__mjnFolderPicked($json)", null
            )
        }
    }
}
KT
echo "written: $BRIDGE"
else
echo "bridge already exists: $BRIDGE"
fi

# ── 2. 覆写 MainActivity（注册 launcher + 注入 WebView）─────────────────────
if ! grep -q "FolderPickerBridge.register" "$MAIN"; then
cat > "$MAIN" << 'KT'
package com.mjnexusreader.app

import android.os.Bundle
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Android 目录批量扫描导入：注册系统目录选择器（必须在 STARTED 前）。
    FolderPickerBridge.register(this)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    // 注入 WebView 引用，供目录枚举结果经 evaluateJavascript 回传前端。
    FolderPickerBridge.attachWebView(webView)
  }
}
KT
echo "patched: $MAIN"
else
echo "MainActivity already injected, nothing to do"
fi
