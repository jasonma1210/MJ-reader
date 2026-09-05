import { create } from "zustand";
import { settingsService } from "../services/settingsService";

export type Lang = "zh-CN" | "en";

interface MeState {
  name: string;
  isGuest: boolean;
  language: Lang;
  syncEnabled: boolean;
  syncStatus: string;
  load: () => Promise<void>;
  setLanguage: (l: Lang) => void;
  setSyncEnabled: (v: boolean) => void;
}

export const useMeStore = create<MeState>((set) => ({
  name: "Reader",
  isGuest: true,
  language: "zh-CN",
  syncEnabled: false,
  syncStatus: "—",

  load: async () => {
    const status = await settingsService.getSyncStatus();
    set({
      syncEnabled: status.enabled,
      syncStatus: status.lastSyncAt
        ? new Date(status.lastSyncAt).toLocaleString()
        : "—",
    });
  },

  setLanguage: (language) => set({ language }),
  setSyncEnabled: (syncEnabled) => set({ syncEnabled }),
}));
