import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  FileUp,
  Upload,
  Package,
  Loader2,
  Download,
  CheckCircle2,
  AlertTriangle,
} from "lucide-react";
import { SettingsPageShell } from "../../components/shell/SettingsPageShell";
import { Button } from "../../components/ui/Button";
import {
  ankiService,
  type AnkiImportReport,
  type AnkiPreview,
} from "../../services/ankiService";
import { toast } from "../../utils/toast";
import { logError } from "../../utils/logError";
import { cn } from "../../utils/cn";

/** 导入流程阶段：0=未选 1=预览中 2=预览完成 3=导入中 4=结果 */
type ImportPhase = 0 | 1 | 2 | 3 | 4;

/** 全维审查#12：Anki 复习资产导入/导出接口
 *  导入：选 .apkg → 预览（不写库）→ 确认写入 flashcards。
 *  导出：本机闪卡全量导出 .apkg（deck 名可自定义）。 */
export function AnkiPage() {
  const { t } = useTranslation();

  // 导入
  const [phase, setPhase] = useState<ImportPhase>(0);
  const [pickedPath, setPickedPath] = useState("");
  const [preview, setPreview] = useState<AnkiPreview | null>(null);
  const [report, setReport] = useState<AnkiImportReport | null>(null);

  // 导出
  const [deckName, setDeckName] = useState("MJNexus Deck");
  const [exporting, setExporting] = useState(false);

  /** 系统对话框选 .apkg 并预览（不写库） */
  const pickAndPreview = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: false,
        filters: [{ name: "Anki Deck", extensions: ["apkg"] }],
      });
      if (!selected) return;
      const path = typeof selected === "string" ? selected : "";
      if (!path) return;
      setPhase(1);
      setPickedPath(path);
      setPreview(null);
      setReport(null);
      const p = await ankiService.previewApkg(path);
      if (!p) {
        setPhase(0);
        toast(t("anki.importFailed"));
        return;
      }
      setPreview(p);
      setPhase(2);
    } catch (e) {
      logError("AnkiPage.pick", e);
      setPhase(0);
      toast(t("anki.importFailed"));
    }
  };

  const handleImport = async () => {
    if (!pickedPath) return;
    setPhase(3);
    try {
      const result = await ankiService.importApkg(pickedPath, null);
      if (result) {
        setReport(result);
        setPhase(4);
        toast(t("anki.importDone"));
      } else {
        setPhase(2);
        toast(t("anki.importFailed"));
      }
    } catch (e) {
      logError("AnkiPage.import", e);
      setPhase(2);
      toast(`${t("anki.importFailed")}: ${String((e as Error)?.message ?? e)}`);
    }
  };

  const doExport = async () => {
    setExporting(true);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const target = await save({
        defaultPath: `${deckName || "MJNexus Deck"}.apkg`,
        filters: [{ name: "Anki Deck", extensions: ["apkg"] }],
      });
      if (!target) return;
      const result = await ankiService.exportApkg(target, deckName || "MJNexus Deck");
      if (result && result.exported > 0) {
        toast(t("anki.exportDone"));
      } else {
        toast(t("anki.exportFailed"));
      }
    } catch (e) {
      logError("AnkiPage.export", e);
      toast(`${t("anki.exportFailed")}: ${String((e as Error)?.message ?? e)}`);
    } finally {
      setExporting(false);
    }
  };

  return (
    <SettingsPageShell title={t("anki.title")}>
      <div className="flex flex-col gap-4 p-4">
        <p className="text-xs leading-relaxed text-ink-muted">{t("anki.hint")}</p>

        {/* ===== 导入 ===== */}
        <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4">
          <div className="mb-3 flex items-center gap-2">
            <Package className="h-4 w-4 text-ink-muted" />
            <h2 className="text-sm font-bold text-ink">{t("anki.importTitle")}</h2>
          </div>

          {phase !== 1 && phase !== 3 && (
            <div className="flex items-center gap-2">
              <Button variant="secondary" iconLeft={<FileUp className="h-4 w-4" />} onClick={pickAndPreview}>
                {t("anki.pickFile")}
              </Button>
              <span className="text-xs text-ink-muted">{t("anki.pickHint")}</span>
            </div>
          )}

          {/* 预览/导入中 */}
          {(phase === 1 || phase === 3) && (
            <div className="flex items-center gap-2 text-sm text-ink-muted">
              <Loader2 className="h-4 w-4 animate-spin" />
              {phase === 1 ? t("anki.previewTitle") : t("anki.importing")}
            </div>
          )}

          {/* 预览详情 */}
          {phase === 2 && preview && (
            <div className="mt-3 flex flex-col gap-3">
              <div className="grid grid-cols-2 gap-2 rounded-md bg-paper-soft p-3 text-sm">
                <div className="text-ink-muted">{t("anki.deck")}</div>
                <div className="truncate text-right font-medium text-ink">{preview.deckName}</div>
                <div className="text-ink-muted">{t("anki.totalNotes")}</div>
                <div className="text-right font-medium text-ink">{preview.totalNotes}</div>
                <div className="text-ink-muted">{t("anki.models")}</div>
                <div className="truncate text-right font-medium text-ink">
                  {preview.models.map((m) => m.name).join(", ") || "—"}
                </div>
                <div className="text-ink-muted">{t("anki.tags")}</div>
                <div className="truncate text-right font-medium text-ink">
                  {preview.tags.slice(0, 6).join(", ") || "—"}
                  {preview.tags.length > 6 ? "…" : ""}
                </div>
              </div>

              {preview.hasCloze && (
                <div className="flex items-center gap-1.5 text-xs text-ai">
                  <AlertTriangle className="h-3.5 w-3.5" />
                  {t("anki.isCloze")}
                </div>
              )}

              <div className="text-xs font-semibold text-ink-muted">{t("anki.sampleTitle")}</div>
              {preview.sampleNotes.length === 0 ? (
                <p className="text-sm text-ink-muted">{t("anki.noNotes")}</p>
              ) : (
                <div className="flex flex-col gap-2">
                  {preview.sampleNotes.map((note) => (
                    <div key={note.id} className="rounded-md border border-line p-3 text-sm">
                      <div className="mb-1 font-semibold text-ink">
                        {note.fields[0] || "—"}
                      </div>
                      {note.fields[1] && (
                        <div className="whitespace-pre-wrap text-ink-soft">{note.fields[1]}</div>
                      )}
                    </div>
                  ))}
                </div>
              )}

              <Button iconLeft={<Upload className="h-4 w-4" />} onClick={handleImport}>
                {t("anki.confirmImport")}（{preview.totalNotes}）
              </Button>
            </div>
          )}

          {/* 导入结果 */}
          {phase === 4 && report && (
            <div className="mt-3 flex flex-col gap-3">
              <div className="flex items-center gap-2 text-sm font-semibold text-accent">
                <CheckCircle2 className="h-4 w-4" />
                {t("anki.importReport")}
              </div>
              <div className="grid grid-cols-3 gap-2 rounded-md bg-paper-soft p-3 text-center text-sm">
                <div>
                  <div className="text-lg font-bold text-accent">{report.imported}</div>
                  <div className="text-xs text-ink-muted">{t("anki.imported")}</div>
                </div>
                <div>
                  <div className="text-lg font-bold text-ink">{report.skipped}</div>
                  <div className="text-xs text-ink-muted">{t("anki.skipped")}</div>
                </div>
                <div>
                  <div className="text-lg font-bold text-ink">{report.durationMs}ms</div>
                  <div className="text-xs text-ink-muted">{t("anki.durationMs")}</div>
                </div>
              </div>
              {report.errors.length > 0 && (
                <details className="text-xs text-ink-muted">
                  <summary className="cursor-pointer font-medium">
                    {t("anki.errorsTitle")}（{report.errors.length}）
                  </summary>
                  <div className="mt-2 max-h-32 overflow-y-auto">
                    {report.errors.map((e, i) => (
                      <div key={i} className="truncate py-0.5">{e}</div>
                    ))}
                  </div>
                </details>
              )}
              <Button
                variant="secondary"
                onClick={() => {
                  setPhase(0);
                  setPreview(null);
                  setReport(null);
                  setPickedPath("");
                }}
              >
                {t("anki.back")}
              </Button>
            </div>
          )}
        </section>

        {/* ===== 导出 ===== */}
        <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4">
          <div className="mb-3 flex items-center gap-2">
            <Download className="h-4 w-4 text-ink-muted" />
            <h2 className="text-sm font-bold text-ink">{t("anki.exportTitle")}</h2>
          </div>
          <div className="flex flex-col gap-2">
            <label className="text-xs text-ink-muted" htmlFor="anki-deck-name">
              {t("anki.exportDeck")}
            </label>
            <input
              id="anki-deck-name"
              className={cn(
                "h-10 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3",
                "text-sm text-ink outline-none focus:border-accent",
              )}
              value={deckName}
              onChange={(e) => setDeckName(e.target.value)}
            />
            <Button variant="secondary" iconLeft={<Download className="h-4 w-4" />} onClick={doExport} disabled={exporting}>
              {exporting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {t("anki.exportBtn")}
            </Button>
          </div>
        </section>
      </div>
    </SettingsPageShell>
  );
}