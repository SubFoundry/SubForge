import { create } from "zustand";

type CoreUiState = {
  loading: boolean;
  setLoading: (loading: boolean) => void;
};

export const useCoreUiStore = create<CoreUiState>((set) => ({
  loading: false,
  setLoading: (loading) => set({ loading }),
}));