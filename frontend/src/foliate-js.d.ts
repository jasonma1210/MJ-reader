// foliate-js 未提供类型声明；以模块声明让其解析为 any，避免 tsc 报 TS7016。
// 运行时由 documentLoader 动态导入并按真实 API 调用。
declare module "foliate-js";
