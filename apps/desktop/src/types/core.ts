export type CoreStatus = {
  running: boolean;
  baseUrl: string;
  version?: string;
  pid?: number;
};

export type CoreApiResponse = {
  status: number;
  headers: Record<string, string>;
  body: string;
};