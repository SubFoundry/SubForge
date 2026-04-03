import type {
  ProfileListResponse,
  ProfileResponse,
  RotateProfileExportTokenResponse,
} from "../../types/core";
import { requestJson } from "./client";

export async function fetchProfiles(): Promise<ProfileListResponse> {
  return requestJson<ProfileListResponse>("GET", "/api/profiles");
}

export async function createProfile(input: {
  name: string;
  description?: string;
  sourceIds: string[];
}): Promise<ProfileResponse> {
  return requestJson<ProfileResponse>("POST", "/api/profiles", {
    name: input.name,
    description: input.description,
    source_ids: input.sourceIds,
  });
}

export async function updateProfile(
  profileId: string,
  input: {
    name?: string;
    description?: string | null;
    sourceIds?: string[];
  },
): Promise<ProfileResponse> {
  return requestJson<ProfileResponse>(
    "PUT",
    `/api/profiles/${encodeURIComponent(profileId)}`,
    {
      name: input.name,
      description: input.description,
      source_ids: input.sourceIds,
    },
  );
}

export async function deleteProfile(
  profileId: string,
): Promise<{ deleted: boolean; id: string }> {
  return requestJson<{ deleted: boolean; id: string }>(
    "DELETE",
    `/api/profiles/${encodeURIComponent(profileId)}`,
  );
}

export async function rotateProfileExportToken(
  profileId: string,
): Promise<RotateProfileExportTokenResponse> {
  return requestJson<RotateProfileExportTokenResponse>(
    "POST",
    `/api/tokens/${encodeURIComponent(profileId)}/rotate`,
  );
}
