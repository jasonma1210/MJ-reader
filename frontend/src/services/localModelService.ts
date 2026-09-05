// 端侧推理模型服务（2026-09-04）：新前端首次接线 local_model 命令族。
// 命令随 llamacpp feature 门控注册——iOS 包未编译该 feature，invoke 会 reject，
// 上层页面（OnDevicePage）须捕获并降级为「能力不可用」提示。
//
// 后端契约（src-tauri/src/commands/local_model.rs）：
// - 进度事件 `local-model-download-progress`：{modelId, downloaded, total, speed, status, resumable}
// - status: not_downloaded / downloading / ready / enabled
// - 断点续传：下载失败/取消保留 .part，重新调用 download 即续传

import { CMD, invoke, isTauri } from "./tauri";

/** 归一化模型卡片（搜索 / 推荐统一结构，对应后端 ModelCard） */
export interface ModelCard {
  repoId: string;
  name: string;
  /** "modelscope" | "hf-mirror" | "huggingface" */
  source: string;
  downloads: number | null;
  likes: number | null;
  pipelineTag: string | null;
  tags: string[];
  updatedAt: string | null;
  curated: boolean;
  paramRange: string | null;
  paramSizeB: number | null;
  agentCapability: string | null;
  /** 2026-09-04：目标平台标签（"ios"|"android"|"desktop"；后端按 target_os 已过滤） */
  platforms?: string[];
  /** 2026-09-04：中文简介（精选清单专属；搜索结果为 null） */
  description?: string | null;
}

/** 仓库文件变体（对应后端 ModelFile） */
export interface ModelFile {
  repoId: string;
  fileName: string;
  /** "gguf" | "projector" | "mlx" */
  fileKind: string;
  quant: string | null;
  sizeBytes: number;
  downloadUrl: string;
  mirrorUrl: string | null;
  modelscopeUrl: string | null;
}

/** 本地模型行（对应后端 LocalModelView） */
export interface LocalModelView {
  id: string;
  name: string;
  source: string;
  repoId: string;
  fileName: string;
  quant: string;
  sizeBytes: number;
  /** "llm" | "projector" | "mlx" */
  modelKind: string;
  localPath: string | null;
  /** not_downloaded / downloading / ready / enabled */
  status: string;
  enabled: boolean;
  downloadedAt: number | null;
  recommended: boolean;
  description: string;
  modelscopeUrl: string | null;
  downloadProgress: null;
  isCatalog: boolean;
}

/** 下载进度事件载荷 */
export interface DownloadProgressEvent {
  modelId: string;
  downloaded: number;
  total: number;
  /** MB/s */
  speed: number;
  /** starting / downloading / completed / error / canceled */
  status: string;
  resumable: boolean;
}

/** 逐文件下载请求（对应后端 ModelFileDownloadRequest） */
export interface ModelFileDownloadRequest {
  repoId: string;
  modelName: string;
  fileName: string;
  fileKind: string;
  quant: string | null;
  sizeBytes: number;
  source: string;
  downloadUrl: string;
  mirrorUrl: string | null;
  modelscopeUrl: string | null;
}

function guard(): void {
  if (!isTauri()) {
    throw new Error("Tauri 环境不可用");
  }
}

/** 本地模型列表（含下载中/已完成/目录项） */
export function listLocalModels(): Promise<LocalModelView[]> {
  guard();
  return invoke<LocalModelView[]>(CMD.listLocalModels);
}

/** 预设模型下载 / 断点续传（source: huggingface | hf-mirror | modelscope） */
export function downloadLocalModel(modelId: string, source: string): Promise<string> {
  guard();
  return invoke<string>(CMD.downloadLocalModel, { modelId, source });
}

/** 取消下载（.part 保留供续传） */
export function cancelLocalModelDownload(modelId: string): Promise<void> {
  guard();
  return invoke<void>(CMD.cancelLocalModelDownload, { modelId });
}

/** 删除本地模型文件与记录 */
export function deleteLocalModel(modelId: string): Promise<void> {
  guard();
  return invoke<void>(CMD.deleteLocalModel, { modelId });
}

/** 启用模型（设为当前端侧推理模型） */
export function enableLocalModel(modelId: string): Promise<void> {
  guard();
  return invoke<void>(CMD.enableLocalModel, { modelId });
}

/** 禁用模型 */
export function disableLocalModel(modelId: string): Promise<void> {
  guard();
  return invoke<void>(CMD.disableLocalModel, { modelId });
}

/** 端侧推理设备档位与可用性（2026-09-05 内存门槛门禁） */
export interface LocalLlmDeviceStatus {
  /** 是否开放端侧推理（iOS ≤6GB / Android ≤8GB 为 false） */
  supported: boolean;
  /** 探测到的总内存（GB） */
  ramGb: number;
  /** 档位标识：ios-high / ios-mid / android-high / android-mid / desktop / unsupported */
  tier: string;
  /** 允许的最大模型体积（GB；桌面端无限时为 0） */
  maxModelGb: number;
  /** 不开放时的原因（由后端给出，前端直接展示；开放时为 null） */
  reason: string | null;
}

/** 查询端侧推理设备档位与是否放行（入口门禁用，不触发模型加载） */
export function getLocalLlmDeviceStatus(): Promise<LocalLlmDeviceStatus> {
  guard();
  return invoke<LocalLlmDeviceStatus>(CMD.getLocalLlmDeviceStatus);
}

/** 运行时状态行（对应后端 LocalModelRuntimeRow） */
export interface LocalModelRuntime {
  modelId: string | null;
  /** unloaded / loading / loaded / inferring */
  state: string;
  loadedAt: number | null;
  lastUsedAt: number | null;
  idleSeconds: number;
  tokensPerSec: number | null;
  memoryMb: number | null;
}

/** 查询运行时状态（模型是否已加载进内存） */
export function getLocalModelRuntime(): Promise<LocalModelRuntime | null> {
  guard();
  return invoke<LocalModelRuntime | null>(CMD.getLocalModelRuntime);
}

/**
 * 显式加载模型（2026-09-04 用户裁定）：加载进内存并常驻（不随推理结束关闭），
 * 单选生效（provider 切端侧）；空闲 1 分钟由后端自动卸载。返回人读结果。
 */
export function loadLocalModel(modelId: string): Promise<string> {
  guard();
  return invoke<string>(CMD.loadLocalModel, { modelId });
}

/** 加载测试：加载核心 + 超短推理验证通路，一次暴露权重/量化/Metal 深层问题。 */
export function testLocalModel(modelId: string): Promise<string> {
  guard();
  return invoke<string>(CMD.testLocalModel, { modelId });
}

/** 模型搜索（source: auto | modelscope | huggingface） */
export function searchLocalModels(
  query: string,
  source: string,
  page: number,
  pageSize: number,
): Promise<{ sourceUsed: string; models: ModelCard[]; hasMore: boolean; nextPage: number }> {
  guard();
  return invoke(CMD.searchLocalModels, { query, source, page, pageSize });
}

/** 推荐模型精选清单（静态，无网络） */
export function listRecommendedModels(): Promise<ModelCard[]> {
  guard();
  return invoke<ModelCard[]>(CMD.listRecommendedModels);
}

/** 仓库文件清单（includeSafetensors=true 时含 MLX 权重） */
export function listModelFiles(
  repoId: string,
  source: string,
  includeSafetensors: boolean,
): Promise<ModelFile[]> {
  guard();
  return invoke<ModelFile[]>(CMD.listModelFiles, { repoId, source, includeSafetensors });
}

/** 逐文件下载（搜索/推荐结果入口，复用断点续传链路） */
export function downloadModelFile(request: ModelFileDownloadRequest): Promise<string> {
  guard();
  return invoke<string>(CMD.downloadModelFile, { request });
}

/** 仓库 README（对应后端 ModelReadme；markdown 截断 16KB） */
export interface ModelReadme {
  repoId: string;
  source: string;
  markdown: string;
  truncated: boolean;
}

/** 获取仓库 README（模型介绍展示用，2026-09-04） */
export function getModelReadme(repoId: string, source: string): Promise<ModelReadme> {
  guard();
  return invoke<ModelReadme>(CMD.getModelReadme, { repoId, source });
}
