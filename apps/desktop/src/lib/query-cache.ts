import type {
  ProfileItem,
  ProfileListResponse,
  SourceListResponse,
  SystemStatusResponse,
} from "../types/core";

export function upsertSourceItem(
  cache: SourceListResponse | undefined,
  item: SourceListResponse["sources"][number],
): SourceListResponse {
  if (!cache) {
    return { sources: [item] };
  }

  const index = cache.sources.findIndex((entry) => entry.source.id === item.source.id);
  if (index < 0) {
    return { sources: [item, ...cache.sources] };
  }

  const next = [...cache.sources];
  next[index] = item;
  return { sources: next };
}

export function removeSourceItem(
  cache: SourceListResponse | undefined,
  sourceId: string,
): SourceListResponse | undefined {
  if (!cache) {
    return cache;
  }
  return { sources: cache.sources.filter((entry) => entry.source.id !== sourceId) };
}

export function patchSourceItem(
  cache: SourceListResponse | undefined,
  sourceId: string,
  patch: {
    name?: string;
    status?: string;
    config?: Record<string, unknown>;
    updatedAt?: string;
  },
): SourceListResponse | undefined {
  if (!cache) {
    return cache;
  }

  return {
    sources: cache.sources.map((entry) => {
      if (entry.source.id !== sourceId) {
        return entry;
      }

      return {
        ...entry,
        source: {
          ...entry.source,
          name: patch.name ?? entry.source.name,
          status: patch.status ?? entry.source.status,
          updated_at: patch.updatedAt ?? entry.source.updated_at,
        },
        config: patch.config ?? entry.config,
      };
    }),
  };
}

export function upsertProfileItem(
  cache: ProfileListResponse | undefined,
  item: ProfileItem,
): ProfileListResponse {
  if (!cache) {
    return { profiles: [item] };
  }

  const index = cache.profiles.findIndex((entry) => entry.profile.id === item.profile.id);
  if (index < 0) {
    return { profiles: [item, ...cache.profiles] };
  }

  const next = [...cache.profiles];
  next[index] = item;
  return { profiles: next };
}

export function removeProfileItem(
  cache: ProfileListResponse | undefined,
  profileId: string,
): ProfileListResponse | undefined {
  if (!cache) {
    return cache;
  }
  return { profiles: cache.profiles.filter((entry) => entry.profile.id !== profileId) };
}

export function patchProfileItem(
  cache: ProfileListResponse | undefined,
  profileId: string,
  patch: {
    name?: string;
    description?: string | null;
    sourceIds?: string[];
    exportToken?: string | null;
    updatedAt?: string;
  },
): ProfileListResponse | undefined {
  if (!cache) {
    return cache;
  }

  return {
    profiles: cache.profiles.map((entry) => {
      if (entry.profile.id !== profileId) {
        return entry;
      }
      return {
        ...entry,
        profile: {
          ...entry.profile,
          name: patch.name ?? entry.profile.name,
          description:
            patch.description !== undefined ? patch.description : entry.profile.description,
          updated_at: patch.updatedAt ?? entry.profile.updated_at,
        },
        source_ids: patch.sourceIds ?? entry.source_ids,
        export_token:
          patch.exportToken !== undefined ? patch.exportToken : entry.export_token,
      };
    }),
  };
}

export function patchSystemStatus(
  cache: SystemStatusResponse | undefined,
  patch: Partial<SystemStatusResponse>,
): SystemStatusResponse | undefined {
  if (!cache) {
    return cache;
  }
  return {
    activeSources: patch.activeSources ?? cache.activeSources,
    totalNodes: patch.totalNodes ?? cache.totalNodes,
    lastRefreshAt: patch.lastRefreshAt ?? cache.lastRefreshAt,
  };
}

