import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "../../lib/api";
import { useCoreUiStore } from "../../stores/core-ui-store";
import type { LogsResponse, RefreshLog, SourceListResponse } from "../../types/core";
import RunsPage from "./page";

vi.mock("../../lib/api", () => ({
  fetchRefreshLogs: vi.fn(),
  fetchSources: vi.fn(),
}));

const fetchRefreshLogsMock = vi.mocked(api.fetchRefreshLogs);
const fetchSourcesMock = vi.mocked(api.fetchSources);

describe("RunsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useCoreUiStore.setState({ phase: "running" });
    fetchSourcesMock.mockResolvedValue(emptySourceList());
  });

  afterEach(() => {
    cleanup();
    useCoreUiStore.setState({ phase: "idle" });
  });

  it("在运行状态波动后会自动收起失效详情，避免再次自动展开", async () => {
    const firstLog = createLog({
      id: "run-1",
      status: "failed",
      errorCode: "E_HTTP_5XX",
      errorMessage: "upstream unavailable",
    });
    const successLog = createLog({
      id: "run-1",
      status: "success",
      errorCode: null,
      errorMessage: null,
    });
    const thirdLog = createLog({
      id: "run-1",
      status: "failed",
      errorCode: "E_HTTP_4XX",
      errorMessage: "temporarily blocked",
    });
    const responses: LogsResponse[] = [
      createLogsResponse({ logs: [firstLog], total: 1, hasMore: false, offset: 0 }),
      createLogsResponse({ logs: [successLog], total: 1, hasMore: false, offset: 0 }),
      createLogsResponse({ logs: [thirdLog], total: 1, hasMore: false, offset: 0 }),
    ];

    let callIndex = 0;
    fetchRefreshLogsMock.mockImplementation(async (options) => {
      expect(options?.includeScriptLogs).toBe(true);
      const index = Math.min(callIndex, responses.length - 1);
      callIndex += 1;
      return responses[index];
    });

    const { queryClient } = renderRunsPage();

    await screen.findByRole("button", { name: "查看详情" });
    await userEvent.click(screen.getByRole("button", { name: "查看详情" }));
    expect(screen.getByText("E_HTTP_5XX")).toBeTruthy();

    await act(async () => {
      await queryClient.invalidateQueries({ queryKey: ["runs", "logs"] });
    });

    await waitFor(() => {
      expect(fetchRefreshLogsMock).toHaveBeenCalledTimes(2);
    });
    await waitFor(() => {
      expect(screen.queryByText("E_HTTP_5XX")).toBeNull();
    });
    expect(screen.queryByRole("button", { name: "收起详情" })).toBeNull();

    await act(async () => {
      await queryClient.invalidateQueries({ queryKey: ["runs", "logs"] });
    });

    await waitFor(() => {
      expect(fetchRefreshLogsMock).toHaveBeenCalledTimes(3);
    });
    await screen.findByRole("button", { name: "查看详情" });
    expect(screen.queryByRole("button", { name: "收起详情" })).toBeNull();
    expect(screen.queryByText("E_HTTP_4XX")).toBeNull();
  });

  it("在日志列表切换后会清理已失效的展开项", async () => {
    fetchRefreshLogsMock.mockImplementation(async (options) => {
      if ((options?.offset ?? 0) > 0) {
        return createLogsResponse({ logs: [], total: 0, hasMore: false, offset: 12 });
      }
      if (fetchRefreshLogsMock.mock.calls.length <= 1) {
        return createLogsResponse({
          logs: [
            createLog({
              id: "run-first",
              sourceName: "Source-First",
              status: "failed",
              errorCode: "E_INTERNAL",
              errorMessage: "first failed",
            }),
          ],
          total: 1,
          hasMore: false,
          offset: 0,
        });
      }
      return createLogsResponse({
        logs: [createLog({ id: "run-second", sourceName: "Source-Second", status: "success" })],
        total: 1,
        hasMore: false,
        offset: 0,
      });
    });

    const { queryClient } = renderRunsPage();

    await screen.findByText("Source-First");
    await userEvent.click(screen.getByRole("button", { name: "查看详情" }));
    expect(screen.getByText("E_INTERNAL")).toBeTruthy();

    await act(async () => {
      await queryClient.invalidateQueries({ queryKey: ["runs", "logs"] });
    });

    await waitFor(() => {
      expect(fetchRefreshLogsMock).toHaveBeenCalledTimes(2);
    });
    await screen.findByText("Source-Second");
    expect(screen.queryByText("first failed")).toBeNull();
    expect(screen.queryByRole("button", { name: "收起详情" })).toBeNull();
  });
});

function renderRunsPage(): { queryClient: QueryClient } {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchOnWindowFocus: false,
        gcTime: 0,
      },
    },
  });

  render(
    <QueryClientProvider client={queryClient}>
      <RunsPage />
    </QueryClientProvider>,
  );

  return { queryClient };
}

function createLogsResponse({
  logs,
  total,
  hasMore,
  offset,
}: {
  logs: RefreshLog[];
  total: number;
  hasMore: boolean;
  offset: number;
}): LogsResponse {
  return {
    logs,
    pagination: {
      limit: 12,
      offset,
      total,
      hasMore,
    },
  };
}

function createLog(
  overrides?: Partial<RefreshLog>,
): RefreshLog {
  return {
    id: overrides?.id ?? "run-default",
    sourceId: overrides?.sourceId ?? "source-1",
    sourceName: overrides?.sourceName ?? "Source-1",
    triggerType: overrides?.triggerType ?? "manual",
    status: overrides?.status ?? "success",
    startedAt: overrides?.startedAt ?? "2026-04-04T00:00:00Z",
    finishedAt: overrides?.finishedAt ?? "2026-04-04T00:00:08Z",
    nodeCount: overrides?.nodeCount ?? 8,
    errorCode: overrides?.errorCode ?? null,
    errorMessage: overrides?.errorMessage ?? null,
    scriptLogs: overrides?.scriptLogs ?? [],
  };
}

function emptySourceList(): SourceListResponse {
  return { sources: [] };
}
