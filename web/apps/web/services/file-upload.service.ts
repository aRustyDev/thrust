/**
 * Copyright (c) 2023-present Plane Software, Inc. and contributors
 * SPDX-License-Identifier: AGPL-3.0-only
 * See the LICENSE file for details.
 */

import type { AxiosRequestConfig } from "axios";
import axios from "axios";
// plane imports
import type { TFileUploadRequestOptions } from "@plane/types";
// services
import { APIService } from "@/services/api.service";

export class FileUploadService extends APIService {
  private cancelSource: any;

  constructor() {
    super("");
  }

  /**
   * Uploads a file to the specified signed URL.
   *
   * POST sends the multipart form built from the policy fields; PUT sends the raw file with the
   * headers that were signed. Pass `requestOptions` from `getFileUploadRequestOptions` — for PUT
   * the headers are part of the signature, so sending different ones fails with 403.
   */
  async uploadFile(
    url: string,
    data: FormData | File,
    requestOptions?: TFileUploadRequestOptions,
    uploadProgressHandler?: AxiosRequestConfig["onUploadProgress"]
  ): Promise<void> {
    // oxlint-disable-next-line import/no-named-as-default-member -- CancelToken is accessed via the default export at runtime
    this.cancelSource = axios.CancelToken.source();
    const { method = "POST", headers = { "Content-Type": "multipart/form-data" } } = requestOptions ?? {};
    const config: AxiosRequestConfig = {
      headers,
      cancelToken: this.cancelSource.token,
      withCredentials: false,
      onUploadProgress: uploadProgressHandler,
    };
    const request = method === "PUT" ? this.put(url, data, config) : this.post(url, data, config);
    return request
      .then((response) => response?.data)
      .catch((error) => {
        // oxlint-disable-next-line import/no-named-as-default-member -- isCancel is accessed via the default export at runtime
        if (axios.isCancel(error)) {
          console.log(error.message);
        } else {
          throw error?.response?.data;
        }
      });
  }

  cancelUpload() {
    this.cancelSource.cancel("Upload canceled");
  }
}
