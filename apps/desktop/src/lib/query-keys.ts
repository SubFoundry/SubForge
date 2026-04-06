export const queryKeys = {
  plugins: {
    all: ["plugins"] as const,
  },
  sources: {
    all: ["sources"] as const,
    pluginSchema: (pluginId: string) => ["sources", "plugin-schema", pluginId] as const,
  },
  profiles: {
    all: ["profiles"] as const,
  },
  dashboard: {
    systemStatus: ["dashboard", "system-status"] as const,
    logsRecent: ["dashboard", "logs", "recent"] as const,
    logsFailed: ["dashboard", "logs", "failed"] as const,
    logsRoot: ["dashboard", "logs"] as const,
  },
  runs: {
    sources: ["runs", "sources"] as const,
    logsRoot: ["runs", "logs"] as const,
    logs: (filters: { status: string; source: string; page: number }) =>
      ["runs", "logs", filters.status, filters.source, filters.page] as const,
  },
} as const;

