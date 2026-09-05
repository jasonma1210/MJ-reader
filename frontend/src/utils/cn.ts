import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** 合并 className：clsx 处理条件类，tailwind-merge 消解冲突的工具类 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
