"use client";

import { createContext, useContext } from "react";

export type DetailCrumb = {
  label: string;
  seasonKey?: string;
  reset: () => void;
};

type DetailStore = {
  detail: DetailCrumb | null;
  setDetail: (detail: DetailCrumb | null) => void;
};

export const DetailContext = createContext<DetailStore>({
  detail: null,
  setDetail: () => {},
});

export function useDetail() {
  return useContext(DetailContext);
}
