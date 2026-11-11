/**
 * Copyright (c) 2023-present Plane Software, Inc. and contributors
 * SPDX-License-Identifier: AGPL-3.0-only
 * See the LICENSE file for details.
 */

import axios, { isCancel } from "axios";
// plane imports
import type { TFileUploadRequestOptions } from "@plane/types";
// api service
import { APIService } from "../api.service";

/**
 * Service class for handling file upload operations
 * Handles file uploads
 * @extends {APIService}
 */
export class FileUploadService extends APIService {
  private cancelSource: any;

  constructor() {
    super("");
  }

  /**
   * Uploads a file to the specified signed URL
   *
   * POST sends the multipart form built from the policy fields; PUT sends the raw file with the
   * headers that were signed. Pass `requestOptions` from `getFileUploadRequestOptions` — for PUT
   * the headers are part of the signature, so sending different ones fails with 403.
   *
   * @param {string} url - The URL to upload the file to
   * @param {FormData | File} data - The form data (POST) or raw file (PUT) to upload
   * @param {TFileUploadRequestOptions} requestOptions - method and signed headers
   * @returns {Promise<void>} Promise resolving to void
   * @throws {Error} If the request fails
   */
  async uploadFile(url: string, data: FormData | File, requestOptions?: TFileUploadRequestOptions): Promise<void> {
    // axios v1 exports CancelToken as a TYPE only; the runtime value lives on the default
    // export, so this rule's named-import suggestion does not compile (TS2693).
    // oxlint-disable-next-line import/no-named-as-default-member
    this.cancelSource = axios.CancelToken.source();
    const { method = "POST", headers = { "Content-Type": "multipart/form-data" } } = requestOptions ?? {};
    const config = {
      headers,
      cancelToken: this.cancelSource.token,
      withCredentials: false,
    };
    const request = method === "PUT" ? this.put(url, data, config) : this.post(url, data, config);
    return request
      .then((response) => response?.data)
      .catch((error) => {
        if (isCancel(error)) {
          console.log(error.message);
        } else {
          throw error?.response?.data;
        }
      });
  }

  /**
   * Cancels the upload
   */
  cancelUpload() {
    this.cancelSource.cancel("Upload canceled");
  }
}
