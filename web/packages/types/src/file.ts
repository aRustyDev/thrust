/**
 * Copyright (c) 2023-present Plane Software, Inc. and contributors
 * SPDX-License-Identifier: AGPL-3.0-only
 * See the LICENSE file for details.
 */

import type { EFileAssetType } from "./enums";

export type TFileMetaDataLite = {
  name: string;
  // file size in bytes
  size: number;
  type: string;
};

export type TFileEntityInfo = {
  entity_identifier: string;
  entity_type: EFileAssetType;
};

export type TFileMetaData = TFileMetaDataLite & TFileEntityInfo;

/**
 * Which presigned upload flavour the server handed back.
 * "POST" is the default (AWS S3, MinIO). "PUT" is for stores without presigned POST support —
 * notably Cloudflare R2, which rejects a presigned POST with 501 NotImplemented.
 */
export type TFileUploadMethod = "POST" | "PUT";

/** Method and headers the upload request must use for a given signed URL. */
export type TFileUploadRequestOptions = {
  method: TFileUploadMethod;
  headers: Record<string, string>;
};

export type TFileSignedURLResponse = {
  asset_id: string;
  asset_url: string;
  upload_data: {
    /** Absent on older servers, which are always POST. */
    method?: TFileUploadMethod;
    url: string;
    /** Populated for POST; empty for PUT, where the key is signed into the URL. */
    fields: Partial<{
      "Content-Type": string;
      key: string;
      "x-amz-algorithm": string;
      "x-amz-credential": string;
      "x-amz-date": string;
      policy: string;
      "x-amz-signature": string;
    }>;
    /** PUT only: headers that were SIGNED and must be sent verbatim or the store returns 403. */
    headers?: Record<string, string>;
  };
};

export type TDuplicateAssetData = {
  entity_id: string;
  entity_type: EFileAssetType;
  project_id?: string;
  asset_ids: string[];
};

export type TDuplicateAssetResponse = Record<string, string>; // asset_id -> new_asset_id
