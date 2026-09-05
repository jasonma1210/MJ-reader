#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""A1 知识包 PC 编译器（本地优先单机架构 · 未实现项 A1）。

把课程文档（Markdown）在 PC 侧批处理为「标准化结构知识包」，产物经同步 / 拷贝入端，
交给端侧 Rust A2 存储检索层（services/knowledge_pack.rs）导入。

产物 JSON schema 与端侧 `KnowledgePackInput` 严格对齐（camelCase）：
    {
      "subject": …,          // 学科
      "title": …,            // 包标题（课程/章节名）
      "description": …,
      "version": "1.0.0",    // 差分更新（A4）版本号
      "sections": [          // 章节/单元
        {
          "title": …,
          "knowledge": [{"name", "desc"}],   // 概念，对齐 breakdown_prompt 的 textbook.concept
          "formulas": [ {"name","content","condition"} ],
          "examPoints": [ {"content","frequency"} ],
          "easyMistakes": [ {"content","hint"} ],
          "memorySkills": [ "…" ],
          "prerequisites": [ "…" ],          // 前置知识
          "controversies": [ "…" ]           // 争议/易混淆
        }
      ],
      "faqs": [ {"question","answer","keywords":[…] } ]  // A3 离线兜底
    }

工作流二选一：
  * 有 LLM API Key（--api-key）：把原文 + 拆书模板字段要求发给 LLM，输出严格 JSON。
  * 无 Key：走确定性结构化兜底（解析 Markdown 特征行），质量较低但零依赖可用。

仅用 Python 标准库，无第三方依赖（LLM 走 urllib）。用法见文件尾 __main__ 帮助。
"""

import argparse
import json
import os
import re
import sys
import urllib.request
from typing import Any


DEFAULT_MODEL = "gpt-4o-mini"
DEFAULT_MAX_TOKENS = 4000


# ---------------------------------------------------------------------------
# Markdown 结构化兜底抽取（no-LLM）
# ---------------------------------------------------------------------------

def _split_sections(text: str):
    """按一级/二级标题切章节，返回 [(title, raw_block_str)]。"""
    lines = text.splitlines()
    sections: list[tuple[str, str]] = []
    cur: tuple[str, list[str]] | None = None
    for ln in lines:
        m = re.match(r"^\s{0,3}(#{1,2})\s+(.*)$", ln)
        if m:
            if cur is not None:
                sections.append((cur[0], "\n".join(cur[1])))
            cur = (m.group(2).strip(), [])
        else:
            if cur is not None:
                cur[1].append(ln)
    if cur is not None:
        sections.append((cur[0], "\n".join(cur[1])))
    return sections


def _clean(block_lines: list[str]) -> str:
    return "\n".join(l for l in block_lines if not l.startswith("#")).strip()


def _extract_knowledge(block: str) -> list[dict]:
    """从 `**概念名**：定义` 或 `概念：` 行抽出 concept。"""
    out: list[dict] = []
    for line in block.splitlines():
        m = re.match(r"^\s*[*\-]?\s*(\*\*)?([^*：：]+)(\*\*)?\s*[：:]\s*(.+)$", line)
        if m and len(m.group(2).strip()) >= 2:
            name = m.group(2).strip()
            desc = m.group(4).strip()
            if not re.match(r"^(概念|公式|考点|易错|记忆|口诀|前置|争议)", name):
                out.append({"name": name[:16], "desc": desc[:60]})
    return out


def _collect_tagged(block: str, tag: str) -> list[str]:
    """收集「前缀标签：内容」行，如 考点：…、易错：…、记忆：…、前置：…、争议：…。"""
    out: list[str] = []
    for line in block.splitlines():
        s = line.strip()
        if s.startswith(tag + "：") or s.startswith(tag + ":"):
            out.append(s.split("：", 1)[-1].split(":", 1)[-1].strip()[:60])
    return out


def _fallback_compile(subject: str, title: str, version: str, sections: list[tuple[str, str]], faqs: list[dict]) -> dict:
    """无 LLM 时的确定性兜底：只填空数组/特征行，绝不编造。"""
    pack_sections: list[dict] = []
    for sec_title, sec_text in sections:
        block = _clean(sec_text.splitlines())
        formulas: list[dict] = []
        for f in re.findall(r"(?:公式|定理)：?\s*([^\n]+)", block):
            formulas.append({"name": "", "content": f.strip()[:40], "condition": ""})
        pack_sections.append({
            "title": sec_title,
            "knowledge": _extract_knowledge(block),
            "formulas": formulas,
            "examPoints": [{"content": x, "frequency": "中频"} for x in _collect_tagged(block, "考点")],
            "easyMistakes": [{"content": x, "hint": ""} for x in _collect_tagged(block, "易错")],
            "memorySkills": _collect_tagged(block, "记忆") or _collect_tagged(block, "口诀"),
            "prerequisites": _collect_tagged(block, "前置"),
            "controversies": _collect_tagged(block, "争议"),
        })
    return {
        "subject": subject,
        "title": title,
        "description": f"{title}（PC 结构化兜底，无 LLM 富化）",
        "version": version,
        "sections": pack_sections,
        "faqs": faqs,
    }


# ---------------------------------------------------------------------------
# LLM 编译（OpenAI 兼容）
# ---------------------------------------------------------------------------

_FIELD_REQ = (
    "现在请把下面的课程文档编译为知识包 JSON，字段如下（camelCase）：\n"
    "{\n"
    '  "subject": "学科",\n'
    '  "title": "包标题",\n'
    '  "description": "一句话概述",\n'
    '  "version": "版本号占位(保持输入)",\n'
    '  "sections": [\n'
    '    {\n'
    '      "title": "章节标题",\n'
    '      "knowledge": [{"name": "概念名(4-16字)", "desc": "定义解释(20-60字)"}],\n'
    '      "formulas": [{"name": "公式/定理名", "content": "内容", "condition": "适用条件"}],\n'
    '      "examPoints": [{"content": "考点内容", "frequency": "高频/中频/低频"}],\n'
    '      "easyMistakes": [{"content": "易错点", "hint": "怎么防"}],\n'
    '      "memorySkills": ["口诀/联想"],\n'
    '      "prerequisites": ["前置知识要点"],\n'
    '      "controversies": ["争议/易混淆论点"]\n'
    "    }\n"
    "  ],\n"
    '  "faqs": [{"question": "常见问题", "answer": "预设答案", "keywords": ["匹配关键词"]}]\n'
    "}\n"
    "规则：只输出合法 JSON，不要任何解释/围栏；原文没有的类别给空数组，禁止编造。"
)


def _llm_compile(api_key: str, base_url: str, model: str, subject: str, title: str,
                 version: str, doc_text: str) -> dict:
    payload = {
        "model": model,
        "temperature": 0.3,
        "max_tokens": DEFAULT_MAX_TOKENS,
        "messages": [
            {"role": "system", "content": "你是课程知识包编译专家，输出严格 JSON。"},
            {"role": "user", "content": f"{_FIELD_REQ}\n\n学科：{subject}\n课程：{title}\n版本号：{version or '1.0.0'}\n\n课程文档：\n{doc_text[:28000]}"},
        ],
    }
    url = base_url.rstrip("/")
    if not url.endswith("/chat/completions"):
        url = url + "/chat/completions"
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {api_key}"},
    )
    with urllib.request.urlopen(req, timeout=120) as resp:  # noqa: S310 (用户显式提供 base_url)
        body = json.loads(resp.read().decode("utf-8"))
    content = body["choices"][0]["message"]["content"]
    # 去围栏代码块（```json ... ```）
    m = re.search(r"```(?:json)?\s*(.+?)\s*```", content, re.S)
    if m:
        content = m.group(1)
    parsed = json.loads(content)
    parsed["version"] = version or parsed.get("version", "1.0.0")
    return parsed


# ---------------------------------------------------------------------------
# FAQ 解析（可选 --faq 文件：`## 问题` + `- 答案...`）
# ---------------------------------------------------------------------------

def _parse_faq(path: str) -> list[dict]:
    faqs: list[dict] = []
    if not os.path.exists(path):
        return faqs
    text = open(path, encoding="utf-8").read()
    blocks = re.split(r"^#{1,2}\s+", text, flags=re.M)
    for b in blocks:
        lines = [l for l in b.splitlines() if l.strip()]
        if not lines:
            continue
        question = lines[0].strip()
        answer = "\n".join(l.lstrip("- ").strip() for l in lines[1:] if l.strip())
        if question and answer:
            faqs.append({
                "question": question[:120],
                "answer": answer[:2000],
                "keywords": _default_keywords(question),
            })
    return faqs


def _default_keywords(question: str) -> list[str]:
    # 粗略取问题中的中文连续段与前几个词作为兜底关键词
    words = [w for w in re.split(r"[，。？?、\s]+", question) if w.strip()]
    return words[:4]


# ---------------------------------------------------------------------------
# 入口
# ---------------------------------------------------------------------------

def _safe_name(s: str) -> str:
    s = s.replace(" ", "_").replace("/", "_").replace("\\", "_")
    return s.strip() or "knowledge_pack"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="A1 知识包 PC 编译器（Markdown → 标准化 JSON 知识包）")
    ap.add_argument("--subject", required=True, help="学科，如 数学")
    ap.add_argument("--title", required=True, help="包标题（课程/章节名）")
    ap.add_argument("--input", nargs="+", required=True, help="课程 Markdown 文件（一个文件=一包，多个=多章节按序）")
    ap.add_argument("--output", help="输出 JSON 路径（默认 knowledge_packs/<subject>_<title>.json）")
    ap.add_argument("--faq", help="可选 FAQ Markdown 文件")
    ap.add_argument("--version", default="1.0.0", help="差分更新版本号（A4）")
    # LLM 可选；不给则走结构化兜底
    ap.add_argument("--api-key", help="OpenAI 兼容 API Key；缺省走本地结构化兜底")
    ap.add_argument("--base-url", default="https://api.openai.com/v1", help="OpenAI 兼容 base_url")
    ap.add_argument("--model", default=DEFAULT_MODEL, help="模型名")
    args = ap.parse_args(argv)

    # 拼接全部章节（按 --input 顺序）
    sections: list[tuple[str, str]] = []
    for path in args.input:
        if not os.path.exists(path):
            print(f"[error] 输入文件不存在: {path}", file=sys.stderr)
            return 2
        text = open(path, encoding="utf-8").read()
        if text:
            base = os.path.basename(path)
            sections.append((os.path.splitext(base)[0], text))

    faqs = _parse_faq(args.faq) if args.faq else []

    if args.api_key:
        doc_text = "\n\n".join(f"## {t}\n{b}" for t, b in sections)
        compiled = _llm_compile(args.api_key, args.base_url, args.model,
                                args.subject, args.title, args.version, doc_text)
        compiled.setdefault("faqs", faqs)
    else:
        split_sections: list[tuple[str, str]] = []
        for t, b in sections:
            split_sections.extend(_split_sections(b))
        compiled = _fallback_compile(args.subject, args.title, args.version, split_sections, faqs)

    out = args.output or os.path.join(
        "knowledge_packs", f"{_safe_name(args.subject)}_{_safe_name(args.title)}.json")
    os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
    with open(out, "w", encoding="utf-8") as fh:
        json.dump(compiled, fh, ensure_ascii=False, indent=2)

    secs = len(compiled.get("sections", []))
    print(f"[ok] 知识包已生成: {out}")
    print(f"     章节数={secs}  FAQ数={len(compiled.get('faqs', []))}  version={compiled.get('version')}")
    return 0


if __name__ == "__main__":
    sys.exit(main())