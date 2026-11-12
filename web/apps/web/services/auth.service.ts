/**
 * Copyright (c) 2023-present Plane Software, Inc. and contributors
 * SPDX-License-Identifier: AGPL-3.0-only
 * See the LICENSE file for details.
 */

// types
import { API_BASE_URL } from "@plane/constants";
import type { ICsrfTokenData, IEmailCheckData, IEmailCheckResponse } from "@plane/types";
// helpers
// services
import { APIService } from "@/services/api.service";

export class AuthService extends APIService {
  constructor() {
    super(API_BASE_URL);
  }

  async requestCSRFToken(): Promise<ICsrfTokenData> {
    return this.get("/auth/get-csrf-token/")
      .then((response) => response.data)
      .catch((error) => {
        throw error;
      });
  }

  emailCheck = async (data: IEmailCheckData): Promise<IEmailCheckResponse> =>
    this.post("/auth/email-check/", data, { headers: {} })
      .then((response) => response?.data)
      .catch((error) => {
        throw error?.response?.data;
      });

  async sendResetPasswordLink(data: { email: string }): Promise<any> {
    return this.post(`/auth/forgot-password/`, data)
      .then((response) => response?.data)
      .catch((error) => {
        throw error?.response;
      });
  }

  async setPassword(token: string, data: { password: string }): Promise<any> {
    return this.post(`/auth/set-password/`, data, {
      headers: {
        "X-CSRFTOKEN": token,
      },
    })
      .then((response) => response?.data)
      .catch((error) => {
        throw error?.response?.data;
      });
  }

  async generateUniqueCode(data: { email: string }): Promise<any> {
    return this.post("/auth/magic-generate/", data, { headers: {} })
      .then((response) => response?.data)
      .catch((error) => {
        throw error?.response?.data;
      });
  }

  /**
   * Thrust (Q-R2, D-022): the password form posts JSON where the pin's posts a form and follows its 302. A success resolves with
   * `{ redirect_path }`, the place the 302 would have gone; a refusal rejects with `{ error_code, error_message, email }`, which
   * is what the 302's query would have carried, so that the sign-in page can show it as it does for the pin.
   */
  async passwordAuth(
    mode: "sign-in" | "sign-up",
    data: { email: string; password: string; next_path?: string }
  ): Promise<{ redirect_path: string }> {
    return this.post(`/auth/${mode}/`, data, { headers: {} })
      .then((response) => response?.data)
      .catch((error) => {
        throw error?.response?.data ?? error;
      });
  }

  /**
   * Thrust (Q-R2, D-022, D-036): the sign-out posts JSON, with the CSRF token as the header the double-submit check reads, where the
   * pin's builds a form and follows its 302. The answer says where to go, which is the place the 302 went.
   */
  async signOut(baseUrl: string): Promise<any> {
    const data = await this.requestCSRFToken();
    const csrfToken = data?.csrf_token;

    if (!csrfToken) throw Error("CSRF token not found");

    const response = await this.post(`${baseUrl}/auth/sign-out/`, {}, { headers: { "X-CSRFTOKEN": csrfToken } });
    window.location.assign(response?.data?.redirect_path ?? "/");
  }
}
